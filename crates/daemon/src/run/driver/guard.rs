//! Final review B-I2: `step` under `catch_unwind`. A bug in the reducer must not take
//! the engine's state, its event loop or the daemon's start with it (AGENTS.md rule 3's
//! aim: one bad window, or here one bad run, never takes the daemon down).

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::run::engine::{Effect, EngineState, Event, EventKind, ReplyId, step};

/// What a request whose step panicked is told.
pub(super) const STEP_PANICKED: &str =
    "the run engine failed on this request (a bug; see the daemon log); the runs are as they were";

/// Steps `event` on `state`. On a panic, `state` is put back as it was before the step
/// and the panic's message is returned; the step's effects are dropped with it. The
/// state is cloned first, as `step` itself clones every run to tell what changed.
pub(super) fn guarded_step(
    state: &mut EngineState,
    event: Event,
) -> Result<Vec<crate::run::engine::Effect>, String> {
    let before = state.clone();
    let taken = std::mem::take(state);
    match catch_unwind(AssertUnwindSafe(move || step(taken, event))) {
        Ok((next, fx)) => {
            *state = next;
            Ok(fx)
        }
        Err(panic) => {
            *state = before;
            Err(panic_text(panic.as_ref()))
        }
    }
}

/// F3 review N3: [`effects::prepare`](super::effects::prepare) (which takes the
/// snapshot) under `catch_unwind` too, since it runs on the step's result outside
/// [`guarded_step`], in the event loop and at the daemon's start. On a panic the step's
/// effects are prepared again without their publishes, so its saves and ops still run
/// and only that snapshot is lost; if that panics as well, nothing runs and it is
/// logged, rather than ending the loop or failing every start.
pub(super) fn prepare_guarded(
    state: &EngineState,
    fx: Vec<Effect>,
    now: u64,
) -> Vec<super::effects::Ready> {
    prepare_guarded_with(state, fx, now, super::effects::prepare)
}

fn prepare_guarded_with(
    state: &EngineState,
    fx: Vec<Effect>,
    now: u64,
    prepare: impl Fn(&EngineState, Vec<Effect>, u64) -> Vec<super::effects::Ready>,
) -> Vec<super::effects::Ready> {
    let rest: Vec<Effect> = fx
        .iter()
        .filter(|e| !matches!(e, Effect::Publish { .. }))
        .cloned()
        .collect();
    let panic = match catch_unwind(AssertUnwindSafe(|| prepare(state, fx, now))) {
        Ok(ready) => return ready,
        Err(panic) => panic_text(panic.as_ref()),
    };
    tracing::error!(%panic, "preparing a step's effects panicked; its publish is dropped");
    catch_unwind(AssertUnwindSafe(|| prepare(state, rest, now))).unwrap_or_else(|panic| {
        tracing::error!(panic = %panic_text(panic.as_ref()), "preparing a step's effects panicked again; they are dropped");
        Vec::new()
    })
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with no message".to_string()
    }
}

/// The request an event carries, whose waiting caller is answered when its step
/// panics.
pub(super) fn reply_of(kind: &EventKind) -> Option<ReplyId> {
    match kind {
        EventKind::Start { reply, .. }
        | EventKind::Approve { reply, .. }
        | EventKind::Reject { reply, .. }
        | EventKind::Edit { reply, .. }
        | EventKind::Retry { reply, .. }
        | EventKind::Override { reply, .. }
        | EventKind::Cancel { reply, .. }
        | EventKind::Resume { reply, .. }
        | EventKind::Finish { reply, .. }
        | EventKind::Tool { reply, .. } => Some(*reply),
        EventKind::BaseAdvanced { .. }
        | EventKind::OpDone { .. }
        | EventKind::Signal { .. }
        | EventKind::Delivered { .. }
        | EventKind::Restore { .. }
        | EventKind::Stop
        | EventKind::Tick => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::driver::effects::Ready;

    /// F3 review N3: a panic while preparing a step's effects (the snapshot is taken
    /// there) costs only the publish; the saves and ops still run, and the caller, the
    /// event loop or the daemon's start, carries on.
    #[test]
    fn a_panicking_snapshot_drops_only_the_publish() {
        let state = EngineState::default();
        let fx = vec![
            Effect::Persist {
                run_id: "r".into(),
                urgent: false,
            },
            Effect::Publish { structural: true },
        ];
        let panicky = |_: &EngineState, fx: Vec<Effect>, _: u64| -> Vec<Ready> {
            if fx.iter().any(|e| matches!(e, Effect::Publish { .. })) {
                panic!("snapshot index out of range");
            }
            fx.into_iter()
                .map(|e| match e {
                    Effect::Persist { run_id, .. } => Ready::Dirty(run_id),
                    other => Ready::Effect(other),
                })
                .collect()
        };
        let ready = prepare_guarded_with(&state, fx, 0, panicky);
        assert_eq!(ready.len(), 1);
        assert!(matches!(&ready[0], Ready::Dirty(id) if id == "r"));
        let always = |_: &EngineState, _: Vec<Effect>, _: u64| -> Vec<Ready> { panic!("x") };
        assert!(prepare_guarded_with(&state, vec![], 0, always).is_empty());
    }
}
