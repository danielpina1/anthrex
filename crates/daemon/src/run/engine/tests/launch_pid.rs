//! M8a.25: the first process of a session starts while its `CreateWindow` is still in
//! flight, so its `ProcessStarted` reaches the engine before any round has the window
//! and is dropped. The `Window` result carries that pid instead, so the round records
//! it: decision 28's reconcile examines only a round's recorded pid, and a Claude
//! session has no other process. Found by
//! `e2e_headless_windows_refuse_client_control_while_the_engine_delivers`, whose worker
//! round never had a pid.

use super::fixture::*;
use crate::run::engine::{OpKind, OpResult};

#[test]
fn a_created_window_records_its_first_process() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    fx.complete_prepares();
    let op = fx
        .run()
        .pending_ops
        .values()
        .find(|p| matches!(p.kind, OpKind::CreateWindow { .. }))
        .map(|p| p.op)
        .expect("a CreateWindow");
    fx.done(
        op,
        OpResult::Window {
            window_id: 5,
            pid: Some(4242),
        },
    );
    let round = &fx.task("t1").rounds[0];
    assert_eq!((round.window_id, round.pid), (Some(5), Some(4242)));
}
