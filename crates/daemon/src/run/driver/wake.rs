//! Milestone 9 decision 39, the driver's half: pasting a wake-up into the orchestrator's
//! PTY window, the one run window that takes a paste (M8a decision 29's outbox and its
//! `Effect::Deliver` are unchanged, and no headless window ever gets one). Decision 13's
//! watch of that window is here too, since both read the manager's window list.
//!
//! **When.** An `Effect::WakeOrchestrator` is kept per run, a newer one replacing one not
//! yet delivered. It is delivered once the window is `Idle` or `Done` (never `Working`,
//! never `Attention`, which may be a permission prompt) and no client input reached it
//! for the run's `wake_quiet_secs`; it is looked at again on every tick and every change
//! of the window list. Delivered, it is answered `OrchEvent::OrchestratorWoken` with
//! the effect's revision and note seq.
//!
//! **How.** `ESC [200~`, the text (`\r\n` and `\n` as `\r`, paste markers removed, cut
//! to the contract's `WAKE_MAX_BYTES`), `ESC [201~`; then, after [`SUBMIT_DELAY`], a
//! lone `\r`. Both go through `WindowManager::write_input`, which only queues the bytes
//! for the window's writer thread, and the delay is a `tokio` sleep on a task of its
//! own: no lock is held across it and nothing waits for it (AGENTS.md rule 2).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use proto::{Status, WindowInfo};
use tokio_util::sync::CancellationToken;

use super::RunService;
use crate::manager::WindowManager;
use crate::run::contract::clamp_with;
use crate::run::engine::{EventKind, OpKind, OrchEvent};
use crate::run::orch::contract::{WAKE_CUT_MARKER, WAKE_MAX_BYTES};

/// Decision 39: the paste, then this long, then the `\r` that submits it.
pub const SUBMIT_DELAY: Duration = Duration::from_millis(200);

/// M9.13 review, item 3: an orchestrator window listed `Exited` counts as exited only
/// once it has stayed so this long (two of the driver's 1 s ticks), and never while a
/// restart of it is under way; a window gone from the list counts at once.
pub const EXIT_CONFIRM: Duration = Duration::from_secs(1);

const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

/// Decision 39's bracketed paste of `text`: line ends as `\r`, paste markers removed
/// (again until none is left, so no removal can join one), cut to `WAKE_MAX_BYTES`.
pub fn encode_paste(text: &str) -> Vec<u8> {
    let mut body = text.replace("\r\n", "\r").replace('\n', "\r");
    while body.contains(PASTE_START) || body.contains(PASTE_END) {
        body = body.replace(PASTE_START, "").replace(PASTE_END, "");
    }
    let body = clamp_with(&body, WAKE_MAX_BYTES, WAKE_CUT_MARKER);
    [PASTE_START, &body, PASTE_END].concat().into_bytes()
}

/// Writes the paste, waits [`SUBMIT_DELAY`] holding nothing, then writes the `\r`.
pub async fn deliver_wake(
    manager: &WindowManager,
    window_id: u32,
    text: &str,
) -> anyhow::Result<()> {
    manager.write_input(window_id, &encode_paste(text))?;
    tokio::time::sleep(SUBMIT_DELAY).await;
    manager.write_input(window_id, b"\r")
}

/// One run's wake-up, waiting for its window.
#[derive(Debug, Clone)]
pub(super) struct Pending {
    window_id: u32,
    text: String,
    digest_revision: u64,
    notes_seq: u64,
    quiet: Duration,
    /// Which insert this is (`Wakes::insert`): a check drops only a wake-up it saw
    /// before it read the engine.
    generation: u64,
}

/// The wake-ups not yet delivered, the runs whose delivery is under way, and when each
/// orchestrator window was first seen `Exited` in a row.
#[derive(Default)]
pub(super) struct Wakes {
    pending: std::sync::Mutex<HashMap<String, Pending>>,
    delivering: std::sync::Mutex<HashSet<String>>,
    exited_since: std::sync::Mutex<HashMap<u32, Instant>>,
    next_generation: std::sync::atomic::AtomicU64,
    /// Tests only: run between a check's two reads (the generations, then the engine).
    #[cfg(test)]
    pub(super) between_reads: std::sync::Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl Wakes {
    /// Decision 39's "a read clears too", for the wake-up already built: a digest
    /// answer that held every note of `run_id`'s waiting wake-up (`notes_seq`, as the
    /// engine's `DigestRead` drops them) drops it, so an orchestrator that read the
    /// change by polling is not pasted at once its turn ends.
    pub(super) fn read(&self, run_id: &str, notes_seq: u64) {
        let mut pending = crate::lock(&self.pending);
        if pending
            .get(run_id)
            .is_some_and(|p| p.notes_seq <= notes_seq)
        {
            pending.remove(run_id);
        }
    }
}

impl Wakes {
    /// Keeps `pending` as `run_id`'s wake-up, replacing one not yet delivered, under a
    /// generation of its own.
    fn insert(&self, run_id: String, mut pending: Pending) {
        pending.generation = self.next_generation.fetch_add(1, Ordering::SeqCst);
        crate::lock(&self.pending).insert(run_id, pending);
    }

    /// Each waiting wake-up's generation, taken before a check reads the engine.
    fn generations(&self) -> HashMap<String, u64> {
        crate::lock(&self.pending)
            .iter()
            .map(|(run_id, p)| (run_id.clone(), p.generation))
            .collect()
    }

    /// Drops the waiting wake-ups `seen` says have no live window to go to, or nothing
    /// left to say. This runs from the tick, the window watch and `queue_wake` at once,
    /// so `seen` may be older than a wake-up queued since. Only a wake-up in `judged`,
    /// the generations taken before `seen` was read, is judged: the engine made it in a
    /// step `seen` already reflects (its state is committed before its effects run).
    /// Any other is left for the next check. "No notes" still drops only a wake-up
    /// whose every note the snapshot had seen made (the first fix round's rule, which
    /// the generation now implies; kept as a second guard).
    fn keep_live(&self, seen: &[Seen], judged: &HashMap<String, u64>) {
        crate::lock(&self.pending).retain(|run_id, p| {
            if judged.get(run_id) != Some(&p.generation) {
                return true;
            }
            seen.iter().any(|s| {
                &s.run_id == run_id
                    && s.window_id == p.window_id
                    && s.live
                    && (s.notes || p.notes_seq > s.last_note_seq)
            })
        });
    }
}

/// A wake-up a check may deliver: its run, window, quiet time and generation.
pub(super) struct Waiting {
    run_id: String,
    window_id: u32,
    quiet: Duration,
    generation: u64,
}

impl Wakes {
    /// The waiting wake-ups a check may deliver: only those it judged on its own engine
    /// snapshot (`judged`). One queued since waits for the next check, which judges it
    /// first; `queue_wake`'s own check does so at once.
    fn deliverable(&self, judged: &HashMap<String, u64>) -> Vec<Waiting> {
        crate::lock(&self.pending)
            .iter()
            .filter(|(run_id, p)| judged.get(*run_id) == Some(&p.generation))
            .map(|(run_id, p)| Waiting {
                run_id: run_id.clone(),
                window_id: p.window_id,
                quiet: p.quiet,
                generation: p.generation,
            })
            .collect()
    }

    /// Takes each of `takes` out for delivery, unless it was replaced or dropped since,
    /// or its run's delivery is under way.
    fn take(&self, takes: Vec<Waiting>) -> Vec<(String, Pending)> {
        let mut pending = crate::lock(&self.pending);
        let mut delivering = crate::lock(&self.delivering);
        takes
            .into_iter()
            .filter_map(|w| {
                let same = pending
                    .get(&w.run_id)
                    .is_some_and(|p| p.window_id == w.window_id && p.generation == w.generation);
                if !same || delivering.contains(&w.run_id) {
                    return None;
                }
                let p = pending.remove(&w.run_id)?;
                delivering.insert(w.run_id.clone());
                Some((w.run_id, p))
            })
            .collect()
    }
}

/// An orchestrator window as the manager lists it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listed {
    Gone,
    Exited,
    Restarting,
    Running,
}

/// What the engine says of one run's orchestrator, read under its lock.
struct Seen {
    run_id: String,
    window_id: u32,
    live: bool,
    exited: bool,
    terminal: bool,
    launching: bool,
    launches: u64,
    /// The engine still holds notes for it: a wake-up whose notes a read dropped since
    /// (decision 39, "a read clears too") has nothing left to say.
    notes: bool,
    /// The seq of the newest note the engine had made (`OrchestratorRecord::last_note_seq`).
    last_note_seq: u64,
}

/// Whether `window` takes a paste now: `Idle` or `Done`, and no client input for
/// `quiet`.
fn ready(window: &WindowInfo, last_input: Option<Instant>, quiet: Duration) -> bool {
    matches!(window.status, Status::Idle | Status::Done)
        && last_input.is_none_or(|at| at.elapsed() >= quiet)
}

impl RunService {
    /// `Effect::WakeOrchestrator`: kept for its run, replacing one not yet delivered,
    /// then looked at at once.
    pub(super) fn queue_wake(
        self: &Arc<Self>,
        run_id: String,
        window_id: u32,
        text: String,
        (digest_revision, notes_seq): (u64, u64),
    ) {
        let quiet = crate::lock(&self.state)
            .runs
            .get(&run_id)
            .map_or(0, |run| run.limits.orch.wake_quiet_secs);
        let pending = Pending {
            window_id,
            text,
            digest_revision,
            notes_seq,
            quiet: Duration::from_secs(quiet),
            generation: 0,
        };
        self.wakes.insert(run_id, pending);
        self.check_orchestrators();
    }

    /// Decisions 13 and 39, on every tick and every change of the window list: reports
    /// an orchestrator window that exited (or came back after an exit) to the engine,
    /// drops the wake-ups of orchestrators that are not live, and delivers each one
    /// whose window takes it now. The engine's lock and the manager's are each taken
    /// for one read, never together and never across an await.
    pub(super) fn check_orchestrators(self: &Arc<Self>) {
        // The waiting wake-ups first, then the engine: every wake-up this check may drop
        // was queued before the engine snapshot it is judged on.
        let judged = self.wakes.generations();
        #[cfg(test)]
        if let Some(hook) = crate::lock(&self.wakes.between_reads).as_ref() {
            hook();
        }
        let seen = self.orchestrators_seen();
        let windows = self.manager.list();
        let window = |id: u32| windows.iter().find(|w| w.id == id);
        let listed = |id: u32| match window(id) {
            None => Listed::Gone,
            Some(_) if self.manager.is_restarting(id) => Listed::Restarting,
            Some(w) if w.status == Status::Exited => Listed::Exited,
            Some(_) => Listed::Running,
        };
        let now: Vec<(u32, Listed)> = seen
            .iter()
            .map(|s| (s.window_id, listed(s.window_id)))
            .collect();
        // How long each has been `Exited` in a row.
        let confirmed: HashSet<u32> = {
            let mut since = crate::lock(&self.wakes.exited_since);
            since.retain(|id, _| now.iter().any(|(w, l)| w == id && *l == Listed::Exited));
            now.iter()
                .filter(|(_, l)| *l == Listed::Exited)
                .filter(|(id, _)| {
                    since.entry(*id).or_insert_with(Instant::now).elapsed() >= EXIT_CONFIRM
                })
                .map(|(id, _)| *id)
                .collect()
        };
        for (s, (_, l)) in seen.iter().zip(&now) {
            let exited = *l == Listed::Gone || confirmed.contains(&s.window_id);
            let report = if s.live && exited {
                Some(false)
            } else {
                let running = *l == Listed::Running;
                let back = !s.live && s.exited && running && !s.terminal && !s.launching;
                back.then_some(true)
            };
            if let Some(live) = report {
                self.send(EventKind::Orch(OrchEvent::OrchestratorWindow {
                    run_id: s.run_id.clone(),
                    window_id: s.window_id,
                    live,
                    launch: s.launches,
                }));
            }
        }
        // Which wake-ups the windows take now, read with no lock of ours held.
        let waiting = {
            self.wakes.keep_live(&seen, &judged);
            self.wakes.deliverable(&judged)
        };
        let takes: Vec<Waiting> = waiting
            .into_iter()
            .filter(|w| {
                window(w.window_id).is_some_and(|win| {
                    ready(win, self.manager.last_client_input(w.window_id), w.quiet)
                })
            })
            .collect();
        let due = self.wakes.take(takes);
        for (run_id, p) in due {
            self.deliver(run_id, p);
        }
    }

    /// Every run's orchestrator window, as the engine has it.
    fn orchestrators_seen(&self) -> Vec<Seen> {
        let state = crate::lock(&self.state);
        state
            .runs
            .values()
            .filter_map(|run| {
                let o = run.orch.orchestrator.as_ref()?;
                Some(Seen {
                    run_id: run.id.clone(),
                    window_id: o.window_id?,
                    live: o.live,
                    exited: o.exited_at.is_some(),
                    launches: o.launches,
                    notes: !o.notes.is_empty(),
                    last_note_seq: o.last_note_seq,
                    terminal: run.state.is_terminal(),
                    launching: run.pending_ops.values().any(|p| {
                        matches!(
                            p.kind,
                            OpKind::CreateOrchestrator { .. } | OpKind::RestartOrchestrator { .. }
                        )
                    }),
                })
            })
            .collect()
    }

    /// The paste, on a task of its own; then `OrchestratorWoken`. A failed write is
    /// logged and dropped: the notes stay, and the next change wakes it again.
    fn deliver(self: &Arc<Self>, run_id: String, p: Pending) {
        let service = self.clone();
        tokio::spawn(async move {
            let delivered = deliver_wake(&service.manager, p.window_id, &p.text).await;
            crate::lock(&service.wakes.delivering).remove(&run_id);
            match delivered {
                Ok(()) if !service.stopped.load(Ordering::SeqCst) => {
                    // A wake-up still waiting that holds no newer note was just pasted.
                    let mut pending = crate::lock(&service.wakes.pending);
                    if pending
                        .get(&run_id)
                        .is_some_and(|next| next.notes_seq <= p.notes_seq)
                    {
                        pending.remove(&run_id);
                    }
                    drop(pending);
                    service.send(EventKind::Orch(OrchEvent::OrchestratorWoken {
                        run_id,
                        digest_revision: p.digest_revision,
                        notes_seq: p.notes_seq,
                    }));
                }
                Ok(()) => {}
                Err(error) => {
                    tracing::warn!(run = %run_id, window = p.window_id, %error, "the wake-up was not delivered");
                }
            }
        });
    }

    /// The window list's changes, for decisions 13 and 39 (the tick looks too).
    pub(super) fn watch_windows(self: &Arc<Self>, shutdown: CancellationToken) {
        let mut changes = self.manager.watch();
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    changed = changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        service.check_orchestrators();
                    }
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "wake_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "wake_exit_tests.rs"]
mod exit_tests;
