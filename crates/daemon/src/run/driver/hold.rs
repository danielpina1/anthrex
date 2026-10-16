//! Milestone 9.16: the hold in the driver (decisions 16, 17, 19 and 20, rulings C9,
//! C10 and C28). `brakes.json` is loaded once, before the restore's step and before
//! the event loop's first ([`RunService::brakes_ready`]); the request writes the file
//! first and then steps `EventKind::Hold`, one request at a time (C10); the refusals
//! of decision 19 read the flag with one lookup under the engine lock; the profile
//! service's drain and the distill worker ask the probes here.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use proto::brakes::DispatchHold;
use proto::run_wire::request;
use proto::{RunReply, RunRequest, Runtime};

use super::brakes_io::{self, BRAKES_IO_WAIT};
use super::{RunService, unix_now};
use crate::run::engine::EventKind;
use crate::run::engine::brakes::HOLD_TEXT;

/// Decision 15: who holds when the user asked.
pub const USER_BY: &str = "user";

/// The service's hold bookkeeping.
#[derive(Default)]
pub(super) struct HoldIo {
    /// Set once `brakes.json` was read; its value is whether the hold was off.
    loaded: tokio::sync::OnceCell<bool>,
    /// Whether the load's drain (ruling C9) was asked for.
    drained: AtomicBool,
    /// Ruling C10: held across a request's save and its `EventKind::Hold`, never
    /// across the engine lock.
    write: tokio::sync::Mutex<()>,
    /// Tests only: a save's wait in place of [`BRAKES_IO_WAIT`], and a pause before its
    /// rename (taken by the next save).
    #[cfg(test)]
    pub(super) seam: std::sync::Mutex<(Option<std::time::Duration>, Option<brakes_io::Pause>)>,
}

impl HoldIo {
    /// The next save's wait and pause.
    fn save_seam(&self) -> (std::time::Duration, Option<brakes_io::Pause>) {
        #[cfg(test)]
        {
            let mut seam = crate::lock(&self.seam);
            (seam.0.unwrap_or(BRAKES_IO_WAIT), seam.1.take())
        }
        #[cfg(not(test))]
        (BRAKES_IO_WAIT, None)
    }
}

/// Ruling C9's probe: whether dispatch is held daemon-wide now.
pub type HeldProbe = Arc<dyn Fn() -> bool + Send + Sync>;
/// Ruling C28's probe: whether a session on the runtime may not start now (the hold,
/// or that runtime's quota brake).
pub type BrakeProbe = Arc<dyn Fn(Runtime) -> bool + Send + Sync>;

impl RunService {
    /// Decision 16: `brakes.json` read once and its hold set on the engine state; every
    /// caller waits for that one load. A load that finds the hold off drains the goals
    /// that waited for it (ruling C9).
    pub(super) async fn brakes_ready(&self) {
        let off = *self
            .hold_io
            .loaded
            .get_or_init(|| async {
                let hold = brakes_io::load(&self.ctx.data_dir, BRAKES_IO_WAIT, unix_now()).await;
                let off = hold.is_none();
                // One field set, as `EventKind::Hold` would set it on a state no step
                // has seen yet; the next step's gate and publish follow from it.
                crate::lock(&self.state).brakes.hold = hold;
                off
            })
            .await;
        if off && !self.hold_io.drained.swap(true, Ordering::SeqCst) {
            self.drain_held_goals();
        }
    }

    /// Starts the load at once, when there is a runtime to run it on.
    pub(super) fn start_brakes_load(self: &Arc<Self>) {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let service = self.clone();
            runtime.spawn(async move { service.brakes_ready().await });
        }
    }

    /// Decision 19's lookup: held, or not loaded yet.
    fn dispatch_held(&self) -> bool {
        self.hold_io.loaded.get().is_none() || crate::lock(&self.state).brakes.hold.is_some()
    }

    /// Decision 19: `Err(HOLD_TEXT)` while the hold is on, after the load.
    pub(super) async fn refuse_if_held(&self) -> Result<(), String> {
        self.brakes_ready().await;
        let held = crate::lock(&self.state).brakes.hold.is_some(); // lookup
        if held {
            Err(HOLD_TEXT.to_string())
        } else {
            Ok(())
        }
    }

    /// Decision 19: the requests the hold refuses before any decider, scout or session
    /// starts, answered under their own labels.
    pub(super) async fn held_refusal(&self, req: &RunRequest) -> Option<RunReply> {
        let label = match req {
            RunRequest::Start { .. } => request::START,
            RunRequest::StartGoal { .. } => request::START_GOAL,
            RunRequest::Promote { .. } => request::PROMOTE,
            RunRequest::Iterate { .. } => request::ITERATE,
            _ => return None,
        };
        let refused = self.refuse_if_held().await.err()?;
        Some(RunReply::refused(label, refused))
    }

    /// Decision 17: `anthrex hold on|off`. The file first; a write that fails refuses
    /// and changes nothing. Then `EventKind::Hold`, whose reply is the message, and the
    /// hold as the engine holds it after. Turning it off drains the held goals (C9).
    pub(super) async fn hold_request(&self, on: bool) -> RunReply {
        let _serial = self.hold_io.write.lock().await;
        self.brakes_ready().await;
        let current = crate::lock(&self.state).brakes.hold.clone(); // lookup
        let wanted = match (on, current) {
            (true, Some(first)) => Some(first),
            (true, None) => Some(DispatchHold {
                since: unix_now(),
                by: USER_BY.to_string(),
            }),
            (false, _) => None,
        };
        let (wait, pause) = self.hold_io.save_seam();
        let saved = brakes_io::save_paused(&self.ctx.data_dir, wanted.clone(), wait, pause).await;
        if let Err(error) = saved {
            return RunReply::refused(request::HOLD, format!("the hold was not saved: {error}"));
        }
        let message = match self
            .ask(|reply| EventKind::Hold {
                reply,
                hold: wanted,
            })
            .await
        {
            Ok(message) => message,
            Err(message) => return RunReply::refused(request::HOLD, message),
        };
        let hold = crate::lock(&self.state).brakes.hold.clone(); // lookup
        if hold.is_none() {
            self.drain_held_goals();
        }
        RunReply::Held {
            hold,
            message,
            request_id: None,
        }
    }

    /// Ruling C9: every project's queued goals drained, when the profile service runs.
    fn drain_held_goals(&self) {
        if let Some(adaptation) = self.adaptation.get() {
            adaptation.profiles.drain_all();
        }
    }

    /// Ruling C9: the profile service's probe, over a weak handle (the run service
    /// holds the profile service); `true` until `brakes.json` has loaded. With the run
    /// service gone nothing is held here: the starter itself refuses then.
    pub fn held_probe(self: &Arc<Self>) -> HeldProbe {
        let weak = Arc::downgrade(self);
        Arc::new(move || weak.upgrade().is_some_and(|runs| runs.dispatch_held()))
    }

    /// Ruling C28: the distill worker's probe: the hold, or `runtime`'s quota brake
    /// as the last step's gate saw it; nothing once the run service is gone.
    pub fn distill_probe(self: &Arc<Self>) -> BrakeProbe {
        let weak = Arc::downgrade(self);
        Arc::new(move |runtime| {
            weak.upgrade().is_some_and(|runs| {
                runs.dispatch_held() || !crate::lock(&runs.state).gate.allows(runtime)
            })
        })
    }
}

#[cfg(test)]
#[path = "hold_tests.rs"]
mod tests;
