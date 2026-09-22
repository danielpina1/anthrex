//! Handling a fresh window list from the daemon: status-change toasts and bells
//! (decision 4), the git map's own pruning, the reveal edges decision 15 and 16 name,
//! and refocusing after a window disappears. Split out of `app/mod.rs` per the
//! 600-line rule (`AGENTS.md` hard rule 8) — the same shape `tree.rs` gives
//! `tree/rows.rs` and `app.rs` already gave `app/modal_keys.rs`.

use super::{App, Effect};
use crate::tree;
use proto::{Status, WindowInfo};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

impl App {
    /// `C-b j`/`C-b k`: focus the next or previous window in tree (agent) order,
    /// wrapping around. Moved here from `app/mod.rs` (task M6.10's file-size finding
    /// B) — it is about picking a window from the current list, the same concern
    /// `replace_windows` below handles for a list that just changed.
    pub(super) fn focus_relative(&mut self, delta: isize) -> Vec<Effect> {
        let visible = tree::agent_order(&self.rows());
        if visible.is_empty() {
            return vec![];
        }
        let len = visible.len() as isize;
        let id = if let Some(current) = self
            .focused
            .and_then(|id| visible.iter().position(|candidate| *candidate == id))
        {
            visible[(current as isize + delta).rem_euclid(len) as usize]
        } else {
            let expanded = tree::agent_order(&tree::build(
                &self.windows,
                &crate::tree::TreeState::default(),
            ));
            let current = self
                .focused
                .and_then(|id| expanded.iter().position(|candidate| *candidate == id));
            current
                .and_then(|current| {
                    (1..=expanded.len()).find_map(|offset| {
                        let index = (current as isize + delta.signum() * offset as isize)
                            .rem_euclid(expanded.len() as isize)
                            as usize;
                        visible
                            .contains(&expanded[index])
                            .then_some(expanded[index])
                    })
                })
                .unwrap_or(visible[0])
        };
        self.focus(id)
    }

    /// The daemon never publishes `None` for an unregistered root, so the client prunes its
    /// own `git` map to the current windows' roots on every window-list change.
    fn prune_git(&mut self) {
        let live: HashSet<PathBuf> = self
            .windows
            .iter()
            .filter_map(|w| w.worktree.clone())
            .collect();
        self.git.retain(|root, _| live.contains(root));
    }

    pub(super) fn replace_windows(&mut self, windows: Vec<WindowInfo>) -> Vec<Effect> {
        // `bell.attention` / `bell.done` (decision 4): only a *background* window's
        // transition rings, matching the toast right above it — the focused window is
        // already on screen and needs neither. Decision 37's own last sentence: "It
        // sends at most one bell per `WindowsChanged`" — so `rings` tracks only
        // whether *any* transition in this batch wants one, and a single
        // `Effect::Bell` is pushed after the loop, not once per qualifying window
        // (whole-branch-review Minor m1: two background windows transitioning in one
        // `WindowsChanged` used to queue two bells, two real `0x07` bytes for what the
        // status line still shows as a single event).
        let mut effects = Vec::new();
        let mut rings = false;
        for w in &windows {
            if Some(w.id) == self.focused {
                continue;
            }
            let previous = self
                .windows
                .iter()
                .find(|old| old.id == w.id)
                .map(|old| old.status);
            if previous != Some(w.status) {
                match w.status {
                    Status::Attention => {
                        self.toast(format!("{} needs attention", w.name));
                        rings |= self.settings.bell_attention;
                    }
                    Status::Done => {
                        self.toast(format!("{} finished", w.name));
                        rings |= self.settings.bell_done;
                    }
                    _ => {}
                }
            }
        }
        if rings {
            effects.push(Effect::Bell);
        }
        // One derivation of the outgoing rows serves both readers below: the
        // agent order the focus falls back through, and the keys the reveal
        // compares against.
        let previous_rows = self.rows();
        let previous_order = tree::agent_order(&previous_rows);
        let previous_keys: Vec<_> = previous_rows.iter().map(|row| row.key.clone()).collect();
        let previous_index = self
            .focused
            .and_then(|id| previous_order.iter().position(|candidate| *candidate == id));
        let previous_selection = self.tree.selected.clone();
        self.windows = windows;
        self.prune_git();
        self.windows_received_at = Instant::now();
        self.tree.prune(&self.windows);
        let rows = tree::build(&self.windows, &self.tree);
        self.tree.repair_selection(&rows);
        // Only on the edges decision 15 names, never on every list. The daemon
        // republishes on every status flip and every output event — several
        // times a second while agents work — and revealing unconditionally
        // would snap the canvas back to the selection about as fast as a
        // person can scroll away from it, undoing what decision 16 grants.
        // A change of focus is the third edge and reveals from `focus` itself,
        // which is the only thing that moves it from here.
        if self.tree.selected != previous_selection
            || rows.iter().map(|row| &row.key).ne(previous_keys.iter())
        {
            self.reveal_tree_anchor();
        }

        if let Some(id) = self.pending_focus
            && self.windows.iter().any(|w| w.id == id)
        {
            self.pending_focus = None;
            effects.extend(self.focus(id));
            return effects;
        }
        if self.focused_window().is_none() {
            if let Some(i) = previous_index
                && !self.windows.is_empty()
            {
                let order = tree::agent_order(&self.rows());
                if let Some(id) = order.get(i.min(order.len().saturating_sub(1))).copied() {
                    effects.extend(self.focus(id));
                    return effects;
                }
            }
            effects.extend(self.ensure_focus());
            return effects;
        }
        effects
    }
}
