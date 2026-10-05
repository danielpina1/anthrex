//! The driver's waiting wake-ups (`wake.rs`, split out move-only by the final fix wave):
//! keeping, confirming and covering them, judging them on an engine snapshot, and
//! taking them out for delivery.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{Pending, Seen, Wakes};

impl Wakes {
    /// Keeps `pending` as `run_id`'s wake-up, replacing one not yet delivered, under a
    /// generation of its own.
    pub(super) fn insert(&self, run_id: String, mut pending: Pending) {
        pending.generation = self.next_generation.fetch_add(1, Ordering::SeqCst);
        crate::lock(&self.pending).insert(run_id, pending);
    }

    /// The generation counter now, taken with [`Wakes::generations`]: a paste stamped
    /// below it was made before the check read the engine (D13).
    pub(super) fn epoch(&self) -> u64 {
        self.next_generation.load(Ordering::SeqCst)
    }

    /// D13: forgets each pasted request that a check reading the engine after its
    /// paste (`epoch`) finds the engine no longer holds (that round's), or whose run it
    /// no longer lists.
    pub(super) fn confirm(&self, seen: &[Seen], epoch: u64) {
        crate::lock(&self.pasted).retain(|run_id, (n, stamp)| {
            *stamp >= epoch
                || seen
                    .iter()
                    .any(|s| &s.run_id == run_id && s.request == Some(*n))
        });
    }

    /// After a paste of `p` for `run_id`: a waiting wake-up it covered is removed (one
    /// holding no newer note, or, for a pasted request, the request's re-emission), and
    /// a pasted request is remembered until the engine has cleared it (D13), and its
    /// notes as pasted (ruling T20-1).
    pub(super) fn pasted(&self, run_id: &str, p: &Pending) {
        let mut pending = crate::lock(&self.pending);
        let covered = |next: &Pending| match (p.request, next.request) {
            (Some(n), Some(m)) => n == m,
            (None, Some(_)) => false,
            (_, None) => next.notes_seq <= p.notes_seq,
        };
        if pending.get(run_id).is_some_and(covered) {
            pending.remove(run_id);
        }
        self.notes_pasted.remember(run_id, p);
        if let Some(n) = p.request {
            let stamp = self.next_generation.fetch_add(1, Ordering::SeqCst);
            crate::lock(&self.pasted).insert(run_id.to_string(), (n, stamp));
        }
    }

    /// `deliver`'s end: a paste that went out (`pasted`) covers what it covered (D13),
    /// and only then is the run's delivery over, so a check in between finds the run
    /// still delivering, and any check after it finds the paste remembered (fix round
    /// 1, I1). A failed or stopped paste only ends the delivery.
    pub(super) fn delivered(&self, run_id: &str, pasted: Option<&Pending>) {
        if let Some(p) = pasted {
            self.pasted(run_id, p);
        }
        #[cfg(test)]
        if let Some(hook) = crate::lock(&self.between_paste_and_release).as_ref() {
            hook(self);
        }
        crate::lock(&self.delivering).remove(run_id);
    }

    /// The runs no longer `going` (ended or discarded) lose their pasted notes' record.
    pub(super) fn forget_ended(&self, going: &HashSet<String>) {
        self.notes_pasted.forget(going);
    }

    /// Each waiting wake-up's generation, taken before a check reads the engine.
    pub(super) fn generations(&self) -> HashMap<String, u64> {
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
    /// the generation now implies; kept as a second guard). A request wake (D13) is
    /// kept while the engine holds a request, and dropped as already pasted while
    /// [`Wakes::pasted`] remembers its run.
    pub(super) fn keep_live(&self, seen: &[Seen], judged: &HashMap<String, u64>) {
        let mut pending = crate::lock(&self.pending);
        let pasted = crate::lock(&self.pasted);
        pending.retain(|run_id, p| {
            if judged.get(run_id) != Some(&p.generation) {
                return true;
            }
            let pasted_round = pasted.get(run_id).map(|(n, _)| *n);
            if p.request.is_some() && p.request == pasted_round {
                return false;
            }
            seen.iter().any(|s| {
                &s.run_id == run_id
                    && s.window_id == p.window_id
                    && s.live
                    && match p.request {
                        Some(n) => s.request == Some(n),
                        None => s.notes || p.notes_seq > s.last_note_seq,
                    }
            })
        });
    }
}

/// A wake-up a check may deliver: its run, window, quiet time and generation.
pub(super) struct Waiting {
    pub(super) run_id: String,
    pub(super) window_id: u32,
    pub(super) quiet: Duration,
    generation: u64,
}

impl Wakes {
    /// The waiting wake-ups a check may deliver: only those it judged on its own engine
    /// snapshot (`judged`). One queued since waits for the next check, which judges it
    /// first; `queue_wake`'s own check does so at once.
    pub(super) fn deliverable(&self, judged: &HashMap<String, u64>) -> Vec<Waiting> {
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
    /// or its run's delivery is under way; without the notes already pasted, and not at
    /// all with none new left (ruling T20-1).
    pub(super) fn take(&self, takes: Vec<Waiting>) -> Vec<(String, Pending)> {
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
                let p = self.notes_pasted.unpasted(&w.run_id, p)?;
                delivering.insert(w.run_id.clone());
                Some((w.run_id, p))
            })
            .collect()
    }
}
