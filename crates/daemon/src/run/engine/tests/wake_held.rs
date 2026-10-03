//! Whole-branch fix round 2, item 2 (tested in fix round 3): the engine's side of
//! `OrchEvent::WakeHeld`. The run shows the held line while the driver says a wake-up
//! is held at a prompt, and drops it when the driver says so or the wake-up is
//! delivered (`OrchestratorWoken`).

use super::fixture::*;
use super::promote::promoted;
use crate::run::engine::{EventKind, OrchEvent};
use crate::run::orch::WAKE_HELD;

fn attention(fx: &Fixture) -> Vec<String> {
    crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0]
        .attention
        .clone()
}

fn held(fx: &mut Fixture, held: bool) {
    fx.next(EventKind::Orch(OrchEvent::WakeHeld {
        run_id: RUN_ID.into(),
        held,
    }));
}

#[test]
fn a_held_wake_shows_until_it_is_delivered_or_released() {
    let mut fx = promoted();
    assert!(!attention(&fx).iter().any(|a| a == WAKE_HELD));
    held(&mut fx, true);
    assert!(
        attention(&fx).iter().any(|a| a == WAKE_HELD),
        "{:?}",
        attention(&fx)
    );
    // Delivered: the line goes.
    fx.next(EventKind::Orch(OrchEvent::OrchestratorWoken {
        run_id: RUN_ID.into(),
        digest_revision: fx.run().orch.digest_rev,
        notes_seq: 0,
        request: None,
    }));
    assert!(!attention(&fx).iter().any(|a| a == WAKE_HELD));
    // Released by the driver (the wake-up was dropped, or the prompt ended).
    held(&mut fx, true);
    held(&mut fx, false);
    assert!(!attention(&fx).iter().any(|a| a == WAKE_HELD));
    // In memory only: a saved run carries no held line.
    held(&mut fx, true);
    let json = serde_json::to_value(fx.run()).unwrap();
    let back: crate::run::model::Run = serde_json::from_value(json).unwrap();
    assert!(!back.orch.wake_held);
}
