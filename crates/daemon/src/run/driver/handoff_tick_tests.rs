//! Decision 29 from the tick: `on_tick` runs the live check (task 9's concern 4), and a
//! live read that hangs holds its one blocking thread, never one more a check (task 9
//! review I1). On `chain_goal_tests.rs`' rig, as the rest of `handoff_chain_tests.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use proto::Runtime;

use super::{ANSWER, ChainRig, PREV, finish, session_start, transcript};

/// Marks [`PREV`] running with its orchestrator live, so the live check measures it.
fn running(rig: &ChainRig) {
    let mut state = crate::lock(&rig.s.state);
    let prev = state.runs.get_mut(PREV).unwrap();
    prev.state = proto::RunState::Running;
    prev.orch.orchestrator.as_mut().unwrap().live = true;
}

/// Removing `check_live` from `on_tick` fails this: the tick alone reports the fill.
#[tokio::test(flavor = "multi_thread")]
async fn on_tick_runs_the_live_check() {
    let rig = ChainRig::new(|_| {}).await;
    let path = transcript(&rig, 184_000);
    session_start(&rig, Runtime::Claude, Some(&path), "sess-a");
    running(&rig);
    let deadline = Instant::now() + ANSWER;
    let fill = loop {
        rig.s.handoff.rearm();
        rig.s.on_tick(crate::run::driver::unix_now()).await;
        let fill = crate::lock(&rig.s.state).runs[PREV].orch.fill.clone();
        if let Some(fill) = fill {
            break fill;
        }
        assert!(Instant::now() < deadline, "the tick reported no fill");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!((fill.tokens, fill.window), (184_000, Some(200_000)));
    finish(rig).await;
}

/// Review I1: a read that outlives its bound keeps its window's read in flight until the
/// blocking read itself ends, so later checks start no second blocking read for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_hung_live_read_holds_one_blocking_thread() {
    let wait = Duration::from_millis(100);
    let rig = ChainRig::with_context(|_| {}, |ctx| ctx.fill_read_cap = Some(wait)).await;
    let started = Arc::new(AtomicUsize::new(0));
    let (release, released) = std::sync::mpsc::channel::<()>();
    let released = Arc::new(std::sync::Mutex::new(released));
    let (count, gate) = (started.clone(), released.clone());
    rig.s.handoff.set_reader(Arc::new(move |_, _| {
        count.fetch_add(1, Ordering::SeqCst);
        // Released (the sender dropped) at the end; the guard is the wait × 100.
        let _ = crate::lock(&gate).recv_timeout(wait * 100);
        None
    }));
    let path = transcript(&rig, 184_000);
    session_start(&rig, Runtime::Claude, Some(&path), "sess-a");
    running(&rig);
    for check in 0..4 {
        rig.s.handoff.rearm();
        rig.s.check_live();
        // The check's task ends at the read's bound; the next check may then start.
        let deadline = Instant::now() + ANSWER;
        while rig.s.handoff.busy.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "check {check} never ended");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            started.load(Ordering::SeqCst) >= 1,
            "check {check} read nothing"
        );
    }
    assert_eq!(
        started.load(Ordering::SeqCst),
        1,
        "a second blocking read started"
    );
    drop(release);
    // Once the hung read ends, the window is read again.
    let deadline = Instant::now() + ANSWER;
    while started.load(Ordering::SeqCst) < 2 {
        assert!(Instant::now() < deadline, "the window was never read again");
        rig.s.handoff.rearm();
        rig.s.check_live();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    finish(rig).await;
}
