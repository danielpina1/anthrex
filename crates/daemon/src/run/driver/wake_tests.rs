//! Task M9.13: decision 39's paste, pure, and its writes outside every lock, against a
//! real manager and a real shell window (never an agent).

use std::time::{Duration, Instant};

use proto::{Runtime, WindowSpec};

use super::*;
use crate::launch::LaunchGate;
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::orch::contract::{WAKE_CUT_MARKER, WAKE_MAX_BYTES};

const START: &[u8] = b"\x1b[200~";
const END: &[u8] = b"\x1b[201~";

/// The body of a paste, its brackets checked.
fn body(paste: &[u8]) -> &[u8] {
    assert!(paste.starts_with(START), "{paste:?}");
    assert!(paste.ends_with(END), "{paste:?}");
    &paste[START.len()..paste.len() - END.len()]
}

#[test]
fn encode_paste_wraps_and_normalises() {
    let paste = encode_paste("[anthrex] one\r\ntwo\nthree\rfour");
    assert_eq!(body(&paste), b"[anthrex] one\rtwo\rthree\rfour");
    // Paste markers inside the text are removed, however they are nested, so the text
    // can neither end the paste early nor open a second one.
    let paste = encode_paste("a\x1b[201~b\x1b[200~c\x1b[20\x1b[201~1~d");
    assert_eq!(body(&paste), b"abcd");
    let inner = body(&paste);
    assert!(!inner.windows(START.len()).any(|w| w == START));
    assert!(!inner.windows(END.len()).any(|w| w == END));
    // Nothing else: one paste, no Enter (the `\r` goes after the delay).
    assert_eq!(encode_paste(""), [START, END].concat());
}

#[test]
fn wake_is_clamped() {
    // `WAKE_MAX_BYTES` is the contract's (`run/orch/contract.rs`); a longer text, which
    // `wake_text` never makes, is still cut to it, on a character boundary.
    let long = "é".repeat(WAKE_MAX_BYTES);
    let paste = encode_paste(&long);
    let inner = body(&paste);
    assert!(inner.len() <= WAKE_MAX_BYTES, "{}", inner.len());
    let text = std::str::from_utf8(inner).expect("cut on a character boundary");
    assert!(text.contains(WAKE_CUT_MARKER.trim()), "{text}");
    // A text at the limit is left whole.
    let exact = "x".repeat(WAKE_MAX_BYTES);
    assert_eq!(body(&encode_paste(&exact)), exact.as_bytes());
}

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &std::path::Path) {}
}

/// Decision 39 and AGENTS.md rules 2 and 3: the paste and its `\r` go through the
/// window's writer thread, and the 200 ms before the `\r` is a `tokio` sleep holding no
/// lock: the manager answers `list`, and the engine's lock is free, while it is pending.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wake_writes_happen_outside_every_lock() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let mut config = ManagerConfig::for_tests(socket, "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let runs = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let spec = WindowSpec {
        name: Some("wake-lock".into()),
        runtime: Runtime::Shell,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let info = manager
        .create(spec, dir.path().to_path_buf(), None, 80, 24)
        .await
        .expect("a shell window");

    let delivering = manager.clone();
    let started = Instant::now();
    let delivery = tokio::spawn(async move {
        deliver_wake(
            &delivering,
            info.id,
            &encode_paste("[anthrex] run r1 changed"),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(60)).await;
    // Inside the delay: both locks answer at once.
    let asked = Instant::now();
    let listed = tokio::task::spawn_blocking({
        let manager = manager.clone();
        move || manager.list()
    })
    .await
    .unwrap();
    assert!(listed.iter().any(|w| w.id == info.id));
    drop(crate::lock(&runs.state));
    assert!(
        asked.elapsed() < Duration::from_millis(100),
        "{:?}",
        asked.elapsed()
    );
    assert!(!delivery.is_finished(), "the delay was not pending");
    delivery.await.unwrap().expect("delivered");
    assert!(started.elapsed() >= SUBMIT_DELAY, "{:?}", started.elapsed());

    // The window this test started, and nothing else.
    let _ = manager.kill(info.id);
}

/// Decision 39, "a read clears too", in the driver (M9.16, found end to end): a digest
/// answer that held every note of the wake-up still waiting drops that wake-up, so an
/// orchestrator that read the change by polling is not pasted at once its turn ends.
/// A wake-up holding a newer note stays.
#[test]
fn a_digest_read_drops_the_wake_up_it_covered() {
    let wakes = Wakes::default();
    let pending = |notes_seq| Pending {
        window_id: 1,
        text: "[anthrex] Run r1 changed: the user approved the plan.".into(),
        digest_revision: 4,
        notes_seq,
        quiet: Duration::from_secs(1),
        generation: 0,
        request: None,
        notes: Vec::new(),
    };
    wakes.insert("r1".into(), pending(3));
    wakes.insert("r2".into(), pending(3));
    wakes.read("r1", 2);
    assert!(
        crate::lock(&wakes.pending).contains_key("r1"),
        "a newer note"
    );
    wakes.read("r1", 3);
    assert!(!crate::lock(&wakes.pending).contains_key("r1"));
    assert!(
        crate::lock(&wakes.pending).contains_key("r2"),
        "another run's"
    );
}

/// The engine's view of run `r1`'s orchestrator in window 1, as a check reads it.
fn seen(live: bool, notes: bool, last_note_seq: u64) -> Seen {
    Seen {
        run_id: "r1".into(),
        window_id: 1,
        live,
        exited: false,
        terminal: false,
        launching: false,
        launches: 1,
        notes,
        last_note_seq,
        request: None,
        quiet: Duration::from_secs(1),
    }
}

/// A wake-up for `r1`'s window 1 holding the notes up to `notes_seq`.
fn waiting(notes_seq: u64) -> Pending {
    Pending {
        window_id: 1,
        text: "[anthrex] Run r1 changed: t1 blocked (question): which name?.".into(),
        digest_revision: 5,
        notes_seq,
        quiet: Duration::from_secs(1),
        generation: 0,
        request: None,
        notes: Vec::new(),
    }
}

/// M9.16 fix round: a snapshot with no notes drops only a wake-up whose notes it had
/// seen made (and a read then cleared), never one built from a newer note.
#[test]
fn a_stale_snapshot_keeps_a_wake_up_built_from_a_newer_note() {
    let wakes = Wakes::default();
    wakes.insert("r1".into(), waiting(4));
    wakes.keep_live(&[seen(true, false, 3)], &wakes.generations());
    assert!(
        crate::lock(&wakes.pending).contains_key("r1"),
        "the new wake-up was dropped by a snapshot older than its note"
    );
    // A snapshot that saw the note (and a read that cleared it) drops it.
    wakes.keep_live(&[seen(true, false, 4)], &wakes.generations());
    assert!(!crate::lock(&wakes.pending).contains_key("r1"));
    // Notes held: kept.
    wakes.insert("r1".into(), waiting(4));
    wakes.keep_live(&[seen(true, true, 4)], &wakes.generations());
    assert!(crate::lock(&wakes.pending).contains_key("r1"));
}

/// M9.16 fix round 2: `check_orchestrators` runs from the tick, the window watch and
/// `queue_wake` at once, so its engine snapshot may predate a wake-up queued since. In
/// order: a check takes its snapshot while the relaunched orchestrator is not live yet
/// (the daemon-restart note, seq 7, already made); the engine sees it live and queues
/// the wake-up for that same note; then the stale check's pass. The wake-up stays, and
/// the next check, reading the engine after it was queued, judges it.
#[test]
fn a_stale_not_live_snapshot_keeps_a_wake_up_queued_after_it() {
    let wakes = Wakes::default();
    let judged = wakes.generations();
    let stale = [seen(false, true, 7)];
    wakes.insert("r1".into(), waiting(7));
    wakes.keep_live(&stale, &judged);
    assert!(
        crate::lock(&wakes.pending).contains_key("r1"),
        "the wake-up was dropped by a snapshot taken before it was queued"
    );
    // A run missing from a stale snapshot (its window not bound yet) keeps it too.
    wakes.keep_live(&[], &judged);
    assert!(crate::lock(&wakes.pending).contains_key("r1"));
    // Judged on a snapshot read after it was queued, it goes when its window is not
    // live.
    let judged = wakes.generations();
    wakes.keep_live(&[seen(false, true, 7)], &judged);
    assert!(!crate::lock(&wakes.pending).contains_key("r1"));
    // A newer wake-up replacing a judged one is not dropped for the old one.
    wakes.insert("r1".into(), waiting(7));
    let judged = wakes.generations();
    wakes.insert("r1".into(), waiting(8));
    wakes.keep_live(&[seen(false, true, 7)], &judged);
    assert!(crate::lock(&wakes.pending).contains_key("r1"));
}

/// M9.16 fix round 3: a check delivers only the wake-ups it judged on its own engine
/// snapshot. One queued after it took its generations (so after its snapshot may have
/// been read) waits for the next check, which judges it first.
#[test]
fn a_check_delivers_only_the_wake_ups_it_judged() {
    let wakes = Wakes::default();
    let judged = wakes.generations();
    wakes.insert("r1".into(), waiting(7));
    assert!(wakes.deliverable(&judged).is_empty(), "delivered unjudged");
    let judged = wakes.generations();
    let next = wakes.deliverable(&judged);
    assert_eq!(next.len(), 1);
    // Replaced between the pass and the take: the newer one is not taken for it.
    wakes.insert("r1".into(), waiting(8));
    assert!(wakes.take(next).is_empty());
    assert!(crate::lock(&wakes.pending).contains_key("r1"));
    let judged = wakes.generations();
    let taken = wakes.take(wakes.deliverable(&judged));
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].1.notes_seq, 8);
}

/// M9.16 fix round 3: `check_orchestrators` reads the waiting wake-ups' generations
/// before the engine. Between the two reads (a test hook), the engine sees the
/// relaunched orchestrator live and queues the wake-up for its daemon-restart note (the
/// same seq). The check must keep it; with the reads the other way round it judges the
/// new wake-up on the not-live snapshot and drops it.
#[tokio::test]
async fn a_check_reads_the_generations_before_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let runs = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let mut run = crate::run::orch::test_support::run_of(1);
    run.id = "r1".into();
    let mut o = crate::run::orch::test_support::orchestrator();
    // A window no manager lists: nothing is delivered, whatever is kept.
    o.window_id = Some(1);
    o.launch_op = None;
    o.live = false;
    o.notes = vec!["the daemon restarted".into()];
    o.note_seqs = vec![7];
    o.last_note_seq = 7;
    run.orch.orchestrator = Some(o);
    crate::lock(&runs.state).runs.insert("r1".into(), run);
    let service = runs.clone();
    *crate::lock(&runs.wakes.between_reads) = Some(Box::new(move || {
        let mut state = crate::lock(&service.state);
        let o = state.runs.get_mut("r1").unwrap().orch.orchestrator.as_mut();
        o.unwrap().live = true;
        drop(state);
        service.wakes.insert("r1".into(), waiting(7));
    }));
    runs.check_orchestrators();
    assert!(
        crate::lock(&runs.wakes.pending).contains_key("r1"),
        "the wake-up queued during the check was dropped"
    );
}

/// Milestone 9.3 decision 11 (D13): round 2's wake for run `r1`'s window 1, holding the
/// notes up to `notes_seq`, with a request of `chars` characters.
fn request_wake(notes_seq: u64, chars: usize) -> Pending {
    round_request(2, notes_seq, chars)
}

/// Round `n`'s request wake for run `r1`'s window 1 (fix round 1: a request's
/// identity is its round).
fn round_request(n: u32, notes_seq: u64, chars: usize) -> Pending {
    let request = "r".repeat(chars);
    Pending {
        text: crate::run::orch::contract_rounds::round_wake("3f9a", n, n as u16 - 1, &request),
        request: Some(n),
        ..waiting(notes_seq)
    }
}

/// The engine holds round `request`'s request wake for `r1` (`None`: no request).
fn seen_request(request: Option<u32>) -> Seen {
    Seen {
        request,
        ..seen(true, false, 3)
    }
}

/// A full check of `wakes` (a tick or the window watch, its window ready) on an engine
/// holding round `engine`'s request: what it takes to paste.
fn check(wakes: &Wakes, engine: Option<u32>) -> Vec<(String, Pending)> {
    let (judged, epoch) = (wakes.generations(), wakes.epoch());
    let seen = [seen_request(engine)];
    wakes.confirm(&seen, epoch);
    wakes.keep_live(&seen, &judged);
    wakes.take(wakes.deliverable(&judged))
}

/// D13: a round's wake is pasted whole, past the 2 KiB a notes-only wake is cut to, up
/// to its own cap, which holds the longest request fenced, the fixed text and the notes.
#[test]
fn a_round_wake_over_2_kib_is_pasted_whole() {
    use crate::run::orch::contract::wake_text;
    use crate::run::orch::contract_rounds::{REQUEST_WAKE_MAX_BYTES, round_wake};
    let wake = request_wake(3, 3 * 1024);
    assert!(wake.text.len() > WAKE_MAX_BYTES);
    let whole = wake.text.replace('\n', "\r");
    assert_eq!(body(&wake.paste()), whole.as_bytes());
    // The same text as a notes-only wake is cut, as before.
    let notes_only = Pending {
        request: None,
        ..wake.clone()
    };
    assert!(body(&notes_only.paste()).len() <= WAKE_MAX_BYTES);
    // The longest request (four-byte characters, or backticks, whose fence is longest)
    // with the longest notes still fits the cap.
    let notes: Vec<String> = (0..20)
        .map(|i| format!("note {i} {}", "n".repeat(200)))
        .collect();
    for request in [
        "\u{1D11E}".repeat(proto::GOAL_MAX_CHARS),
        "`".repeat(proto::GOAL_MAX_CHARS),
    ] {
        let text = format!(
            "{}\n{}",
            round_wake("3f9a", 20, 32, &request),
            wake_text("engine-test-3f9a", &notes)
        );
        assert!(text.len() <= REQUEST_WAKE_MAX_BYTES, "{}", text.len());
        let pending = Pending {
            text: text.clone(),
            ..wake.clone()
        };
        assert_eq!(body(&pending.paste()), text.replace('\n', "\r").as_bytes());
    }
}

/// D13: a `run_status` read covers notes, never a round's request: its wake waits.
#[test]
fn a_run_status_read_leaves_a_request_wake_waiting() {
    let wakes = Wakes::default();
    wakes.insert("r1".into(), request_wake(3, 10));
    wakes.read("r1", 99);
    assert!(crate::lock(&wakes.pending).contains_key("r1"));
    // Judged on a snapshot with no notes left, the engine still holding the request.
    wakes.keep_live(&[seen_request(Some(2))], &wakes.generations());
    assert!(crate::lock(&wakes.pending).contains_key("r1"));
    // A notes-only paste holding every note does not remove it either.
    wakes.pasted("r1", &waiting(99));
    assert!(crate::lock(&wakes.pending).contains_key("r1"));
    // Once the engine holds no request, it goes; and when the window is not live.
    wakes.keep_live(&[seen_request(None)], &wakes.generations());
    assert!(!crate::lock(&wakes.pending).contains_key("r1"));
    wakes.insert("r1".into(), request_wake(3, 10));
    let not_live = Seen {
        live: false,
        ..seen_request(Some(2))
    };
    wakes.keep_live(&[not_live], &wakes.generations());
    assert!(!crate::lock(&wakes.pending).contains_key("r1"));
}

/// D13 (amended): the engine re-emits a request with every change until it applies
/// `OrchestratorWoken { request: Some(n) }`. A step between `deliver`'s removal and that
/// event re-emits it; it is not pasted twice, whether it arrives while the paste is
/// under way or after it, and a later request is pasted again.
#[test]
fn a_request_wake_is_pasted_once_across_the_woken_race() {
    let wakes = Wakes::default();
    // While the paste is under way.
    wakes.insert("r1".into(), request_wake(3, 10));
    let taken = check(&wakes, Some(2));
    assert_eq!(taken.len(), 1, "deliver's removal");
    wakes.insert("r1".into(), request_wake(3, 10));
    assert!(check(&wakes, Some(2)).is_empty(), "taken while delivering");
    wakes.delivered("r1", Some(&taken[0].1));
    assert!(check(&wakes, Some(2)).is_empty(), "pasted twice");
    // After the paste, before the engine applies `OrchestratorWoken`.
    wakes.insert("r1".into(), request_wake(4, 10));
    assert!(check(&wakes, Some(2)).is_empty(), "pasted twice");
    assert!(!crate::lock(&wakes.pending).contains_key("r1"));
    // The engine cleared it: a later round's request is pasted.
    assert!(check(&wakes, None).is_empty());
    wakes.insert("r1".into(), round_request(3, 5, 10));
    assert_eq!(check(&wakes, Some(3)).len(), 1, "the next round's request");
}

/// Fix round 1, I2 (a): the user rejects round 2 and iterates round 3 while round 2's
/// paste is under way (its 200 ms before the `\r`). Round 2's paste covers only round
/// 2's re-emissions, so round 3's waits and is pasted exactly once.
#[test]
fn a_later_request_inside_the_paste_window_is_pasted_once() {
    let wakes = Wakes::default();
    wakes.insert("r1".into(), round_request(2, 3, 10));
    let a = check(&wakes, Some(2));
    assert_eq!(a.len(), 1);
    // Reject, then iterate: the engine holds round 3's request and emits it.
    wakes.insert("r1".into(), round_request(3, 4, 10));
    assert!(
        check(&wakes, Some(3)).is_empty(),
        "round 2 is still being pasted"
    );
    wakes.delivered("r1", Some(&a[0].1));
    let pending = crate::lock(&wakes.pending).get("r1").map(|p| p.request);
    assert_eq!(
        pending,
        Some(Some(3)),
        "round 2's paste dropped round 3's request"
    );
    let b = check(&wakes, Some(3));
    assert_eq!(b.len(), 1, "round 3's request");
    assert_eq!(b[0].1.request, Some(3));
    wakes.delivered("r1", Some(&b[0].1));
    // Its re-emissions until the engine applies its `OrchestratorWoken`: none pasted.
    wakes.insert("r1".into(), round_request(3, 4, 10));
    assert!(check(&wakes, Some(3)).is_empty(), "round 3 pasted twice");
}

/// Fix round 1, I2 (b): round 2's request is pasted and its `OrchestratorWoken`
/// applied, then the user rejects round 2 and iterates round 3, all before any check
/// saw round 2's request cleared. Round 2's paste is remembered still, but only for
/// round 2: round 3's request is pasted exactly once.
#[test]
fn a_later_request_before_any_check_saw_the_last_cleared_is_pasted_once() {
    let wakes = Wakes::default();
    wakes.insert("r1".into(), round_request(2, 3, 10));
    let a = check(&wakes, Some(2));
    assert_eq!(a.len(), 1);
    wakes.delivered("r1", Some(&a[0].1));
    // The engine: woken (round 2 cleared), reject, iterate round 3; no check between.
    wakes.insert("r1".into(), round_request(3, 4, 10));
    let b = check(&wakes, Some(3));
    assert_eq!(b.len(), 1, "round 3's request was dropped as round 2's");
    let memo = crate::lock(&wakes.pasted).get("r1").copied();
    assert_eq!(
        memo, None,
        "round 2's paste outlived the engine holding round 2"
    );
    wakes.delivered("r1", Some(&b[0].1));
    wakes.insert("r1".into(), round_request(3, 4, 10));
    assert!(check(&wakes, Some(3)).is_empty(), "round 3 pasted twice");
}

/// D13 through `check_orchestrators`: the engine's `request_wake` is what a check
/// reads, so a pasted request's re-emission is dropped while the engine still holds
/// it, and the pasted mark goes once the engine no longer does.
#[tokio::test]
async fn a_check_drops_a_pasted_requests_re_emission() {
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let runs = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let mut run = crate::run::orch::test_support::run_of(1);
    run.id = "r1".into();
    let mut o = crate::run::orch::test_support::orchestrator();
    // A window no manager lists: nothing is delivered, whatever is kept.
    o.window_id = Some(1);
    o.launch_op = None;
    o.live = true;
    run.orch.orchestrator = Some(o);
    run.orch.request_wake = Some("the round's wake".into());
    // The request is the run's current round's (fix round 1).
    let n = run.round();
    crate::lock(&runs.state).runs.insert("r1".into(), run);
    let pending = |runs: &RunService| crate::lock(&runs.wakes.pending).contains_key("r1");
    runs.wakes.insert("r1".into(), round_request(n, 3, 10));
    runs.check_orchestrators();
    assert!(
        pending(&runs),
        "a request the engine holds waits for its window"
    );
    runs.wakes.pasted("r1", &round_request(n, 3, 10));
    assert!(!pending(&runs), "the paste removed its duplicate");
    runs.wakes.insert("r1".into(), round_request(n, 3, 10));
    runs.check_orchestrators();
    assert!(!pending(&runs), "a pasted request's re-emission was kept");
    crate::lock(&runs.state)
        .runs
        .get_mut("r1")
        .unwrap()
        .orch
        .request_wake = None;
    runs.check_orchestrators();
    crate::lock(&runs.state)
        .runs
        .get_mut("r1")
        .unwrap()
        .orch
        .request_wake = Some("next".into());
    runs.wakes.insert("r1".into(), round_request(n, 3, 10));
    runs.check_orchestrators();
    assert!(
        pending(&runs),
        "the next request is kept once the engine cleared the last"
    );
}

/// Fix round 1, I1: `deliver` ends with the paste remembered before the run's delivery
/// is over. A full check (a tick or the window watch) that runs between the two steps,
/// with the engine still holding the request, takes nothing: with the old order (the
/// delivery released first) it would take the re-emission and paste it a second time.
#[test]
fn a_check_between_the_paste_and_its_release_pastes_nothing() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let wakes = Wakes::default();
    wakes.insert("r1".into(), request_wake(3, 10));
    let taken = check(&wakes, Some(2));
    assert_eq!(taken.len(), 1, "the paste under way");
    // The engine re-emits the request while the paste goes out.
    wakes.insert("r1".into(), request_wake(4, 10));
    let between = Arc::new(AtomicUsize::new(usize::MAX));
    let seen_between = between.clone();
    *crate::lock(&wakes.between_paste_and_release) = Some(Box::new(move |w: &Wakes| {
        seen_between.store(check(w, Some(2)).len(), Ordering::SeqCst);
    }));
    wakes.delivered("r1", Some(&taken[0].1));
    assert_eq!(between.load(Ordering::SeqCst), 0, "pasted twice");
    assert!(!crate::lock(&wakes.delivering).contains("r1"), "released");
    assert!(!crate::lock(&wakes.pending).contains_key("r1"));
}
