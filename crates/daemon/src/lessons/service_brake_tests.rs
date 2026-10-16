//! Milestone 9.16 ruling C28: the distill worker starts no distill while the hold is
//! on or the distill helper's runtime is quota-braked; its entries stay queued, and it
//! resumes when the brake lifts. No failover. The probe is injected; the default
//! helpers row is Claude's.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use proto::{DeciderMode, LessonLine, Runtime};

use super::{Rig, is_distilled, lines, passes, queued, until, write};

/// A probe braking whatever `braked` says, recording each runtime it is asked about.
fn probe(
    braked: Arc<Mutex<Vec<Runtime>>>,
    asked: Arc<Mutex<Vec<Runtime>>>,
) -> crate::run::driver::BrakeProbe {
    Arc::new(move |runtime| {
        crate::lock(&asked).push(runtime);
        crate::lock(&braked).contains(&runtime)
    })
}

#[tokio::test]
async fn a_braked_distill_waits_queued_and_resumes() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(&repo, &[queued("r1", super::T)]);
    let braked = Arc::new(Mutex::new(vec![Runtime::Claude]));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let service = rig.start_braked(
        10,
        DeciderMode::Claude,
        Some(probe(braked.clone(), asked.clone())),
    );
    passes(&service, 2).await;
    assert_eq!(rig.calls(), 0, "a distill started while braked");
    // Nothing was written: no attempt, no `gave_up`; the entry stays queued.
    assert_eq!(lines(&repo).len(), 1, "{:?}", lines(&repo));
    assert!(matches!(lines(&repo)[0], LessonLine::Queued(_)));
    assert!(crate::lock(&asked).iter().all(|r| *r == Runtime::Claude));
    assert!(!crate::lock(&asked).is_empty());

    // The other runtime braked does not hold the Claude helper.
    *crate::lock(&braked) = vec![Runtime::Codex];
    until("the distilled line", || is_distilled(&repo, "r1")).await;
    assert_eq!(rig.calls(), 1);
    service.stop();
}

/// The hold (every runtime braked) keeps the worker's first scan's entries too, and
/// the worker resumes on its own once it lifts.
#[tokio::test]
async fn the_hold_stops_the_distill_until_it_lifts() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(&repo, &[queued("r1", super::T), queued("r2", super::T + 1)]);
    let held = Arc::new(AtomicBool::new(true));
    let flag = held.clone();
    let service = rig.start_braked(
        10,
        DeciderMode::Claude,
        Some(Arc::new(move |_| flag.load(Ordering::SeqCst))),
    );
    passes(&service, 2).await;
    assert_eq!(rig.calls(), 0);
    held.store(false, Ordering::SeqCst);
    until("both distilled", || {
        is_distilled(&repo, "r1") && is_distilled(&repo, "r2")
    })
    .await;
    assert_eq!(rig.calls(), 2);
    service.stop();
}
