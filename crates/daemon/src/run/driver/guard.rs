//! Final review B-I2: `step` under `catch_unwind`. A bug in the reducer must not take
//! the engine's state, its event loop or the daemon's start with it (AGENTS.md rule 3's
//! aim: one bad window, or here one bad run, never takes the daemon down).

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::run::engine::{Effect, EngineState, Event, EventKind, ReplyId, step};

/// What a request whose step panicked is told.
pub(super) const STEP_PANICKED: &str =
    "the run engine failed on this request (a bug; see the daemon log); the runs are as they were";

/// Decision 8 (M9.11.8): what a request is told when its step was committed but carrying
/// out its effects panicked. `STEP_PANICKED`'s "the runs are as they were" would be false.
pub(super) const EFFECTS_PANICKED: &str = "the run engine failed while carrying out this request (a bug; see the daemon log); the run may not match what was saved";

/// Steps `event` on `state`. On a panic, `state` is put back as it was before the step
/// and the panic's message is returned; the step's effects are dropped with it. The
/// state is cloned first, as `step` itself clones every run to tell what changed.
pub(super) fn guarded_step(
    state: &mut EngineState,
    event: Event,
) -> Result<Vec<crate::run::engine::Effect>, String> {
    guarded_step_keeping(state, event).map(|(fx, _before)| fx)
}

/// [`guarded_step`], also returning the state as it was before the step (decision 8),
/// so a caller whose effects cannot be prepared can put it back.
fn guarded_step_keeping(
    state: &mut EngineState,
    event: Event,
) -> Result<(Vec<Effect>, EngineState), String> {
    let before = state.clone();
    let taken = std::mem::take(state);
    match catch_unwind(AssertUnwindSafe(move || step(taken, event))) {
        Ok((next, fx)) => {
            *state = next;
            Ok((fx, before))
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
/// logged, rather than ending the loop or failing every start. The event loop uses
/// [`step_prepared`], which also puts the state back then (decision 8); the restore
/// keeps this form, with nothing run.
pub(super) fn prepare_guarded(
    state: &EngineState,
    fx: Vec<Effect>,
    now: u64,
) -> Vec<super::effects::Ready> {
    prepare_guarded_with(state, fx, now, super::effects::prepare).unwrap_or_default()
}

/// `None` when preparing panicked twice, so nothing runs.
fn prepare_guarded_with(
    state: &EngineState,
    fx: Vec<Effect>,
    now: u64,
    prepare: impl Fn(&EngineState, Vec<Effect>, u64) -> Vec<super::effects::Ready>,
) -> Option<Vec<super::effects::Ready>> {
    let rest: Vec<Effect> = fx
        .iter()
        .filter(|e| !matches!(e, Effect::Publish { .. }))
        .cloned()
        .collect();
    let panic = match catch_unwind(AssertUnwindSafe(|| prepare(state, fx, now))) {
        Ok(ready) => return Some(ready),
        Err(panic) => panic_text(panic.as_ref()),
    };
    tracing::error!(%panic, "preparing a step's effects panicked; its publish is dropped");
    match catch_unwind(AssertUnwindSafe(|| prepare(state, rest, now))) {
        Ok(ready) => Some(ready),
        Err(panic) => {
            tracing::error!(panic = %panic_text(panic.as_ref()), "preparing a step's effects panicked again; they are dropped");
            None
        }
    }
}

/// Decision 8 (B3): steps `event` on `state`, then prepares its effects
/// ([`prepare_guarded`]). When preparing panics twice, nothing of the step would run, so
/// `state` is put back as it was before the step and the step is reported as panicked.
pub(super) fn step_prepared(
    state: &mut EngineState,
    event: Event,
    now: u64,
) -> Result<Vec<super::effects::Ready>, String> {
    step_prepared_with(state, event, now, super::effects::prepare)
}

fn step_prepared_with(
    state: &mut EngineState,
    event: Event,
    now: u64,
    prepare: impl Fn(&EngineState, Vec<Effect>, u64) -> Vec<super::effects::Ready>,
) -> Result<Vec<super::effects::Ready>, String> {
    let (fx, before) = guarded_step_keeping(state, event)?;
    match prepare_guarded_with(state, fx, now, prepare) {
        Some(ready) => Ok(ready),
        None => {
            *state = before;
            Err("preparing the step's effects panicked twice; the step is undone".to_string())
        }
    }
}

pub(super) fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with no message".to_string()
    }
}

/// The event's kind, for the log line of a panic in its handling (decision 8).
pub(super) fn kind_name(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::Start { .. } => "Start",
        EventKind::Approve { .. } => "Approve",
        EventKind::Reject { .. } => "Reject",
        EventKind::Edit { .. } => "Edit",
        EventKind::Retry { .. } => "Retry",
        EventKind::Override { .. } => "Override",
        EventKind::Cancel { .. } => "Cancel",
        EventKind::Resume { .. } => "Resume",
        EventKind::BaseAdvanced { .. } => "BaseAdvanced",
        EventKind::Finish { .. } => "Finish",
        EventKind::Tool { .. } => "Tool",
        EventKind::Promote { .. } => "Promote",
        EventKind::OrchestratorUsage { .. } => "OrchestratorUsage",
        EventKind::OpDone { .. } => "OpDone",
        EventKind::Signal { .. } => "Signal",
        EventKind::Delivered { .. } => "Delivered",
        EventKind::Restore { .. } => "Restore",
        EventKind::Stop => "Stop",
        EventKind::Tick => "Tick",
        EventKind::Orch(_) => "Orch",
        EventKind::Delivery(_) => "Delivery",
        EventKind::Iterate { .. } => "Iterate",
        EventKind::DocGate { .. } => "DocGate",
        EventKind::DesignChecked { .. } => "DesignChecked",
        EventKind::PoolNote { .. } => "PoolNote",
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
        | EventKind::Tool { reply, .. }
        | EventKind::Promote { reply, .. }
        | EventKind::Iterate { reply, .. }
        | EventKind::DocGate { reply, .. } => Some(*reply),
        EventKind::Orch(event) => event.reply(),
        EventKind::Delivery(request) => Some(request.reply()),
        EventKind::BaseAdvanced { .. }
        | EventKind::OrchestratorUsage { .. }
        | EventKind::OpDone { .. }
        | EventKind::Signal { .. }
        | EventKind::Delivered { .. }
        | EventKind::Restore { .. }
        | EventKind::DesignChecked { .. }
        | EventKind::PoolNote { .. }
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
        let ready = prepare_guarded_with(&state, fx, 0, panicky).expect("prepared again");
        assert_eq!(ready.len(), 1);
        assert!(matches!(&ready[0], Ready::Dirty(id) if id == "r"));
        let always = |_: &EngineState, _: Vec<Effect>, _: u64| -> Vec<Ready> { panic!("x") };
        assert!(prepare_guarded_with(&state, vec![], 0, always).is_none());
    }

    /// Decision 8 (B3): when preparing a step's effects panics twice, none of them runs,
    /// so the state is put back as it was before the step: memory never holds a step
    /// whose saves did not run. The step here (an approve) changes the run's state, its
    /// revision and its log, each compared on its own.
    #[test]
    fn a_double_panic_in_prepare_puts_the_state_back() {
        use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
        let mut run = run_ok(&plan_with(
            PROFILE,
            &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
        ));
        run.id = "r".into();
        let mut state = EngineState::default();
        let start = Event {
            now: 5,
            kind: EventKind::Start {
                reply: 1,
                run: Box::new(run),
            },
        };
        guarded_step(&mut state, start).expect("the start steps");
        let before = state.clone();
        let approve = Event {
            now: 7,
            kind: EventKind::Approve {
                reply: 2,
                run_id: "r".into(),
            },
        };
        // The step itself does change the run, so a state left stepped is told apart.
        let mut stepped = state.clone();
        guarded_step(&mut stepped, approve.clone()).expect("the approve steps");
        assert_ne!(stepped.runs["r"].state, before.runs["r"].state);

        let always = |_: &EngineState, _: Vec<Effect>, _: u64| -> Vec<Ready> {
            panic!("prepare always panics")
        };
        let prepared = step_prepared_with(&mut state, approve, 7, always);
        assert!(prepared.is_err(), "a double panic must be reported");
        let (now, was) = (&state.runs["r"], &before.runs["r"]);
        assert_eq!(now.state, was.state);
        assert_eq!(now.revision, was.revision);
        assert_eq!(now.log, was.log);
        assert_eq!(state.revision, before.revision);
        assert_eq!(state, before);
    }
}
