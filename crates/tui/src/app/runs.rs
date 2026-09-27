//! Milestone 8c: the client's copy of the run snapshot (task M8c.2, decisions 1, 2 and
//! 34). One pushed `RunsSnapshot`, subscribed once per connection, replaced whole by
//! every push; the daemon's clock as the client sees it; and the toasts for the run
//! replies this client asked for.

use super::{App, Effect};
use crate::tree;
use proto::{AgentRoundInfo, ClientMsg, RunReply, RunRequest, RunsSnapshot};
use std::time::Instant;

/// `App.runs` before the first snapshot arrives: revision 0, no runs.
pub(super) fn no_runs() -> RunsSnapshot {
    RunsSnapshot {
        revision: 0,
        runs: vec![],
        now: 0,
    }
}

/// The longest daemon text a toast shows, in characters, before `…`: a reply's text is
/// the daemon's, and a toast is one status-bar line (review M1).
pub(crate) const TOAST_MAX_CHARS: usize = 300;

/// `text` cut to [`TOAST_MAX_CHARS`] characters plus `…`, on a char boundary.
fn capped(text: &str) -> String {
    let mut out: String = text.chars().take(TOAST_MAX_CHARS).collect();
    if text.chars().nth(TOAST_MAX_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// Decision 34: a refusal can be several lines (`engine/requests.rs` joins a batch's
/// errors with `\n`). Shows the first non-blank line, capped, then ` (+{n} more)` for
/// the other non-blank lines; `None` when there is no text at all.
pub(crate) fn first_line_and_more(text: &str) -> Option<String> {
    let mut lines = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty());
    let first = capped(lines.next()?);
    Some(match lines.count() {
        0 => first,
        more => format!("{first} (+{more} more)"),
    })
}

impl App {
    /// Decision 1: the one `Run(Subscribe)` per connection. `lib.rs` sends it right
    /// after `App::new`, and `on_reconnected` again for every new connection.
    pub fn run_subscription(&mut self) -> Effect {
        self.run_subscribed = true;
        Effect::Send(ClientMsg::Run(RunRequest::Subscribe))
    }

    /// `on_tick`'s share of decision 1: a refused `Run(Subscribe)` (`on_send_failed`
    /// cleared `run_subscribed`) is sent again, once, while connected. Disconnected,
    /// `on_reconnected` sends it instead.
    pub(super) fn retry_run_subscription(&mut self) -> Option<Effect> {
        (self.connected() && !self.run_subscribed).then(|| self.run_subscription())
    }

    /// `on_tick`: every dropped subscription, the focused window's and the runs'.
    pub(super) fn retry_dropped_subscribes(&mut self) -> Vec<Effect> {
        let window = self.retry_dropped_subscribe();
        window
            .into_iter()
            .chain(self.retry_run_subscription())
            .collect()
    }

    pub(crate) fn on_run_reply(&mut self, reply: RunReply) -> Vec<Effect> {
        match reply {
            // Decision 1: every snapshot replaces the last, whatever its revision — a
            // restarted daemon counts from the start again. A push also proves the
            // subscription is live, so a pending retry has nothing left to do.
            RunReply::Snapshot(snapshot) => {
                self.replace_runs(snapshot);
                self.runs_received_at = Instant::now();
                self.run_subscribed = true;
            }
            RunReply::Done { message, .. } => self.toast(capped(&message)),
            RunReply::Refused { request, message } => {
                let text = first_line_and_more(&message);
                self.toast(text.unwrap_or_else(|| format!("{request} refused")));
            }
            RunReply::Started { .. }
            | RunReply::ConfirmNeeded { .. }
            | RunReply::ToolResult { .. }
            | RunReply::Triaged { .. }
            | RunReply::Profile(_)
            | RunReply::Stats(_) => {}
        }
        vec![]
    }

    /// Task M8c.3: the tree follows the snapshot as it follows a window list —
    /// fold state of runs that left is pruned, the selection repaired, and the
    /// selection revealed only when the rows changed (decision 15 of milestone 4.6).
    fn replace_runs(&mut self, snapshot: RunsSnapshot) {
        let previous_keys: Vec<_> = self.rows().into_iter().map(|row| row.key).collect();
        let previous_selection = self.tree.selected.clone();
        self.runs = snapshot;
        self.tree.prune_runs(&self.runs.runs);
        self.tree.prune(&self.windows);
        let rows = tree::build_with_runs(&self.windows, &self.runs.runs, &self.tree);
        self.tree.repair_selection(&rows);
        let changed = rows.iter().map(|row| &row.key).ne(previous_keys.iter());
        if changed || self.tree.selected != previous_selection {
            self.reveal_tree_anchor();
        }
    }

    /// Decision 2: the daemon's unix seconds as this client sees them now — the
    /// snapshot's `now` plus the whole seconds since it arrived.
    pub fn run_now(&self) -> u64 {
        let elapsed = self.runs_received_at.elapsed().as_secs();
        self.runs.now.saturating_add(elapsed)
    }

    /// Decision 2: seconds since `unix_secs` on the daemon's clock; 0 for a time that
    /// has not happened yet.
    pub fn run_age(&self, unix_secs: u64) -> u64 {
        self.run_now().saturating_sub(unix_secs)
    }

    /// Decision 2: rate-limited exactly while the limit ends in the future. The
    /// snapshot's own `rate_limited` flag, true only as of publication, is not read.
    pub fn rate_limited(&self, round: &AgentRoundInfo) -> bool {
        round
            .rate_limited_until
            .is_some_and(|until| until > self.run_now())
    }

    /// Tests move the snapshot's arrival into the past instead of sleeping.
    #[cfg(test)]
    pub(crate) fn set_runs_received_at(&mut self, at: Instant) {
        self.runs_received_at = at;
    }
}
