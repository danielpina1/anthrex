//! The tick and the size handling (milestone 9.11, M9.11.21), moved out of `app/mod.rs`:
//! the debounced resize, the first-size focus, and the 100 ms tick's housekeeping.

use super::{App, Effect, RESIZE_DEBOUNCE, TOAST_TTL};
use crate::tree;
use common::numbers::{self, Zone};
use proto::ClientMsg;
use std::time::Instant;

impl App {
    /// The viewer's zone, for every clock time the client prints (decision 3).
    pub fn zone(&self) -> Zone {
        Zone::Local(self.utc_offset_secs)
    }

    /// `at` (unix seconds) as a local clock time against the daemon's `now`
    /// ([`App::run_now`]): `14:30 +02:00`, `Thu 14:30 +02:00`, `2026-10-12 14:30 +02:00`.
    pub fn local_clock(&self, at: u64) -> String {
        numbers::clock(at, self.run_now(), self.zone())
    }

    /// The renderer calls this with the main inner area after every draw.
    pub fn set_terminal_size(&mut self, cols: u16, rows: u16) -> Vec<Effect> {
        self.set_terminal_size_at(cols, rows, Instant::now())
    }

    /// [`App::set_terminal_size`] at a given instant (milestone 9.11 decision 23): the
    /// debounce of a later size change is measured from `now`.
    pub fn set_terminal_size_at(&mut self, cols: u16, rows: u16, now: Instant) -> Vec<Effect> {
        // The conversation view draws in the same main area, so its interior is this
        // size too; set before the early return so it is right from the first frame.
        self.conversation.set_interior_width(cols);
        if cols == 0 || rows == 0 || (cols, rows) == self.term_size {
            return vec![];
        }
        let first = self.term_size == (0, 0);
        self.term_size = (cols, rows);
        self.parser.screen_mut().set_size(rows, cols);
        if first {
            return self.ensure_focus();
        }
        self.pending_resize = Some(now);
        vec![]
    }

    pub(super) fn ensure_focus(&mut self) -> Vec<Effect> {
        if self.term_size == (0, 0) {
            return vec![];
        }
        if let Some(id) = self.pending_focus
            && self.windows.iter().any(|w| w.id == id)
        {
            self.pending_focus = None;
            return self.focus(id);
        }
        if self.focused_window().is_some() {
            return vec![];
        }
        match tree::focus_fallback(&self.rows(), 0) {
            Some(id) => {
                let effects = self.focus(id);
                self.auto_focus = true;
                effects
            }
            None => {
                self.focused = None;
                self.follow_no_focus()
            }
        }
    }

    /// [`App::on_tick_at`] now, keeping the zone offset it has.
    pub fn on_tick(&mut self) -> Vec<Effect> {
        self.on_tick_at(Instant::now(), self.utc_offset_secs)
    }

    /// Called every 100 ms: advances the spinner, expires toasts, retries a dropped
    /// `Subscribe`, flushes a debounced resize. At a given instant (milestone 9.11
    /// decision 23): the toast's time to live and the resize debounce are measured
    /// against `now`. `utc_offset_secs` is the local zone's offset, which the event loop
    /// reads (milestone 9.19 decision 3, ruling R4); it is stored, never read here.
    pub fn on_tick_at(&mut self, now: Instant, utc_offset_secs: i32) -> Vec<Effect> {
        self.ticked_at = now;
        self.utc_offset_secs = utc_offset_secs;
        let mut effects = self.expire_replies();
        effects.extend(self.screens_tick(now));
        effects.extend(self.tick(now));
        // Milestone 9.11 decision 22 (fix round 1): a dropped-goal status held by the gap.
        effects.extend(self.ask_dropped_due(now));
        effects
    }

    fn tick(&mut self, now: Instant) -> Vec<Effect> {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
        self.forget_stale_panel_state();
        // Milestone 9.11 decision 24: a modal a daemon reply opened over a pressed prefix
        // clears it, so the prefix is not left waiting once the modal closes.
        if self.modal.is_some() {
            self.keymap.clear_pending();
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|t| now.saturating_duration_since(t.at) >= TOAST_TTL)
        {
            self.toast = None;
        }
        // Decision 36: the third of the three ways `C-b Q`'s wait can end — nothing
        // arrived at all within `link::STOPPING_TIMEOUT`.
        self.check_stopping_timeout();
        let retries = self.retry_dropped_subscribes();
        if !retries.is_empty() {
            return retries;
        }
        if self
            .pending_resize
            .is_some_and(|at| now.saturating_duration_since(at) >= RESIZE_DEBOUNCE)
        {
            self.pending_resize = None;
            if let Some(id) = self.focused_pty() {
                let (cols, rows) = self.term_size;
                return vec![Effect::Send(ClientMsg::Resize {
                    window_id: id,
                    cols,
                    rows,
                })];
            }
        }
        // Decision 23: the inspected task's detail, once per task and key.
        self.check_task_detail().into_iter().collect()
    }
}
