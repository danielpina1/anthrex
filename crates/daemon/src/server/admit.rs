//! Milestone 9.14 decisions 16, 21 and 23: the daemon's side of `anthrex admit`. An ask
//! waits on the daemon's test scheduler as `Priority::Agent`, beside the connection
//! loop, and a grant is held in the [`HoldBook`] under `(window, tool_use_id)` until a
//! hook event or the sweep releases it (M9.14.9).
//!
//! The hold book is a `std::sync::Mutex` taken with `crate::lock` and held only to read
//! or change the book: never across an `.await` (AGENTS.md rule 2). A grant leaving the
//! book is dropped after its lock is let go, since dropping it takes the scheduler's.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proto::{ClientMsg, DaemonMsg};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::admit::{ADMIT_LABEL_MAX_CHARS, HoldBook, Release};
use crate::manager::WindowManager;
use crate::run::slots::{Priority, SlotGrant, SlotRequest, TestScheduler, Want};

/// The daemon's admission: its scheduler, the granted holds and the longest a hold may
/// last (`[testing] admit_hold_max_secs`, read once at start; the sweep's, M9.14.9).
pub(crate) struct AdmitService {
    scheduler: Arc<TestScheduler>,
    holds: Mutex<HoldBook<SlotGrant>>,
    #[expect(dead_code, reason = "M9.14.9's sweep reads it")]
    hold_max: Duration,
}

impl AdmitService {
    pub(crate) fn new(scheduler: Arc<TestScheduler>, testing: &config::Testing) -> Arc<Self> {
        Arc::new(AdmitService {
            scheduler,
            holds: Mutex::new(HoldBook::default()),
            hold_max: Duration::from_secs(testing.admit_hold_max_secs),
        })
    }

    /// Answers a `ClientMsg::Admit` on its own task, beside the connection loop (as
    /// `requests::detach` does); an unknown window is an `Error`. `token` is the
    /// connection's, cancelled as its read loop exits (decision 23, C2): a client that
    /// goes away while waiting gives up its place.
    pub(super) fn admit(
        self: &Arc<Self>,
        manager: &WindowManager,
        out: mpsc::Sender<DaemonMsg>,
        token: CancellationToken,
        msg: ClientMsg,
    ) {
        let ClientMsg::Admit { window_id, .. } = &msg else {
            return;
        };
        let known = manager.list().iter().any(|w| w.id == *window_id);
        let service = self.clone();
        drop(tokio::spawn(async move {
            service.answer(known, out, token, msg).await
        }));
    }

    async fn answer(
        &self,
        known: bool,
        out: mpsc::Sender<DaemonMsg>,
        token: CancellationToken,
        msg: ClientMsg,
    ) {
        let ClientMsg::Admit {
            window_id: window,
            tool_use_id,
            slots,
            wait_secs,
            label,
        } = msg
        else {
            return;
        };
        let label: String = label.chars().take(ADMIT_LABEL_MAX_CHARS).collect();
        tracing::info!(window, tool_use_id, ?slots, label, "admit asked");
        if !known {
            let _ = out
                .send(DaemonMsg::Error {
                    request: "admit".into(),
                    message: format!("no window {window}"),
                })
                .await;
            return;
        }
        if let Some(held) = self.held(window, &tool_use_id) {
            tracing::info!(
                window,
                tool_use_id,
                slots = held_slots(&held),
                label,
                "admit already held"
            );
            let _ = out.send(held).await;
            return;
        }
        let request = SlotRequest {
            priority: Priority::Agent,
            critical: false,
            want: slots.map_or(Want::Half, Want::Count),
            exclusive: false,
            label: label.clone(),
        };
        let wait = Duration::from_secs(wait_secs.min(config::ADMIT_WAIT_MAX_SECS));
        let grant = tokio::select! {
            biased;
            _ = token.cancelled() => {
                tracing::info!(window, tool_use_id, ?slots, label, "admit abandoned");
                return;
            }
            grant = self.scheduler.acquire(request) => grant,
            _ = tokio::time::sleep(wait) => {
                tracing::info!(window, tool_use_id, ?slots, label, "admit timed out");
                let _ = out.send(DaemonMsg::AdmitTimedOut).await;
                return;
            }
        };
        // C16: two asks for one key that both waited; the second takes nothing new.
        // C15: otherwise the hold is in the book before `Admitted` is sent.
        let (reply, duplicate) = {
            let mut book = crate::lock(&self.holds);
            match book.get(window, &tool_use_id) {
                Some(held) => (admitted(held), Some(grant)),
                None => {
                    let reply = admitted(&grant);
                    book.insert(window, tool_use_id.clone(), grant, Instant::now());
                    (reply, None)
                }
            }
        };
        let granted = held_slots(&reply);
        if duplicate.is_some() {
            drop(duplicate);
            tracing::info!(
                window,
                tool_use_id,
                slots = granted,
                label,
                "admit already held"
            );
            let _ = out.send(reply).await;
            return;
        }
        tracing::info!(window, tool_use_id, slots = granted, label, "admit granted");
        // Decision 23: a connection that closed before the grant was handed over does
        // not keep it.
        if token.is_cancelled() || out.send(reply).await.is_err() {
            let released =
                crate::lock(&self.holds).release(window, &Release::One(tool_use_id.clone()));
            drop(released);
            tracing::info!(
                window,
                tool_use_id,
                slots = granted,
                label,
                "admit abandoned"
            );
        }
    }

    /// `Admitted` with the held grant's slots, when this tool use already holds one.
    fn held(&self, window: u32, tool_use_id: &str) -> Option<DaemonMsg> {
        crate::lock(&self.holds)
            .get(window, tool_use_id)
            .map(admitted)
    }
}

fn admitted(grant: &SlotGrant) -> DaemonMsg {
    DaemonMsg::Admitted {
        slots: grant.slots(),
        load: grant.load().to_string(),
    }
}

fn held_slots(reply: &DaemonMsg) -> u32 {
    match reply {
        DaemonMsg::Admitted { slots, .. } => *slots,
        _ => 0,
    }
}
