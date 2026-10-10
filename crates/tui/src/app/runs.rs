//! Milestone 8c: the client's copy of the run snapshot (task M8c.2, decisions 1, 2 and
//! 34). One pushed `RunsSnapshot`, subscribed once per connection, replaced whole by
//! every push; the daemon's clock as the client sees it; and the toasts for the run
//! replies this client asked for. Task M8c.6 adds the run view's state (decision 11):
//! opening, leaving and the keys only it has; Enter inside it is `run_enter.rs`. Task
//! M8c.9 adds the plan gate (decisions 32–34): `a`, `x`, `e` and `d`, the edit form's
//! keys and its replies, whose code is in `run_gate.rs`. The client only ever sends the user's own requests, each after
//! a confirm (the `y` of a `Confirm`, or the form's `Enter`).

use super::{App, Effect, TreeInput};
use crate::tree::{self, NodeKey, Row, RunFilter, TreeState};
use crossterm::event::{KeyCode, KeyEvent};
use proto::{
    AgentRoundInfo, ClientMsg, RunReply, RunRequest, RunState, RunsSnapshot, Runtime, WindowInfo,
};
use std::time::Instant;

/// Decision 11: the overview rooted at one run instead of the project tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    pub run_id: String,
    pub filter: RunFilter,
}

/// Decision 11's row list, from the fields apart, so a caller can go on to mutate
/// `App.tree` with the rows in hand: the run view's rows while `view` names a shown
/// run, the project tree's otherwise.
pub fn nav_rows_of<'a>(
    windows: &'a [WindowInfo],
    snapshot: &'a RunsSnapshot,
    state: &TreeState,
    view: Option<&RunView>,
) -> Vec<Row<'a>> {
    let shown = view.and_then(|view| {
        tree::shown_runs(&snapshot.runs)
            .find(|run| run.run_id == view.run_id)
            .map(|run| (run, view.filter))
    });
    match shown {
        Some((run, filter)) => tree::run_rows(run, windows, state, filter),
        None => tree::build_from(windows, snapshot, state),
    }
}

/// Decision 21: `f`'s cycle.
fn next_filter(filter: RunFilter) -> RunFilter {
    match filter {
        RunFilter::All => RunFilter::Running,
        RunFilter::Running => RunFilter::Blocked,
        RunFilter::Blocked => RunFilter::Runtime(Runtime::Claude),
        RunFilter::Runtime(Runtime::Claude) => RunFilter::Runtime(Runtime::Codex),
        RunFilter::Runtime(_) => RunFilter::All,
    }
}

/// Decision 21's labels, as the status bar shows them.
pub fn filter_label(filter: RunFilter) -> &'static str {
    match filter {
        RunFilter::All => "all",
        RunFilter::Running => "running",
        RunFilter::Blocked => "blocked",
        RunFilter::Runtime(Runtime::Claude) => "claude",
        RunFilter::Runtime(Runtime::Codex) => "codex",
        RunFilter::Runtime(Runtime::Shell) => "shell",
    }
}

/// Decision 23: this view's own words for a run's state (`RunState::label` is
/// snake_case).
pub(crate) fn state_text(state: RunState) -> &'static str {
    match state {
        RunState::AwaitingApproval => "awaiting approval",
        RunState::Running => "running",
        RunState::Paused => "paused",
        RunState::Halted => "halted",
        RunState::Complete => "complete",
        RunState::Accepted => "accepted",
        RunState::Discarded => "discarded",
        RunState::Failed => "failed",
        RunState::Planning => "planning",
        RunState::Brainstorming => "brainstorming",
        RunState::Specifying => "specifying",
    }
}

/// `App.runs` before the first snapshot arrives: revision 0, no runs.
pub(super) fn no_runs() -> RunsSnapshot {
    RunsSnapshot {
        revision: 0,
        runs: vec![],
        now: 0,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
        queued_goals: Vec::new(),
    }
}

/// The longest daemon text a toast shows, in characters, before `…`: a reply's text is
/// the daemon's, and a toast is one status-bar line (review M1).
pub(crate) const TOAST_MAX_CHARS: usize = 300;

/// `text` cut to [`TOAST_MAX_CHARS`] characters plus `…`, on a char boundary, with no
/// control, line-separator or bidi character (`safe_text`, milestone 9's M-6).
pub(super) fn capped(text: &str) -> String {
    let head: String = text.chars().take(TOAST_MAX_CHARS).collect();
    let mut out = crate::safe_text::one_line(&head);
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

    /// Milestone 9 decision 2: `request` as a tagged request under a fresh id, which
    /// its reply echoes. Ids count from 1 for the client's life.
    pub(super) fn tagged_request(&mut self, request: RunRequest) -> (u64, Effect) {
        self.next_request_id += 1;
        let id = self.next_request_id;
        (id, Effect::Send(ClientMsg::RunTagged { id, request }))
    }

    /// Decision 44: the run a goal started, opened once a snapshot names it.
    pub(super) fn open_pending_run(&mut self) {
        let Some(id) = self.pending_open.as_ref() else {
            return;
        };
        if tree::shown_runs(&self.runs.runs).any(|run| run.run_id == *id) {
            let id = id.clone();
            self.pending_open = None;
            self.open_run_view(id);
        }
    }

    pub(crate) fn on_run_reply(&mut self, reply: RunReply) -> Vec<Effect> {
        if let Some(effects) = self.route_reply(&reply) {
            return effects;
        }
        match reply {
            // Decision 1: every snapshot replaces the last, whatever its revision — a
            // restarted daemon counts from the start again. A push also proves the
            // subscription is live, so a pending retry has nothing left to do.
            RunReply::Snapshot(snapshot) => {
                self.replace_runs(snapshot);
                self.runs_received_at = Instant::now();
                self.run_subscribed = true;
                return self.focus_off_idle();
            }
            // Decision 34: an edit's reply ends a submitting form — closed on `Done`,
            // its error row filled on `Refused`. Every other reply is a toast.
            // Milestone 9 decision 2: a form's reply is the one carrying its request's
            // id, never one that merely names the same request.
            RunReply::Done {
                message,
                request_id,
                ..
            } => {
                // Milestone 9.3 decision 32: and the iterate dialog's.
                if self.form_waiting_on(request_id).is_some()
                    || self.iterate_waiting_on(request_id).is_some()
                {
                    self.modal = None;
                }
                self.toast(capped(&message));
            }
            RunReply::Refused {
                request,
                message,
                request_id,
            } => {
                // Decision 23: the task detail's refusal is the panel's, not a toast.
                if self.on_task_detail_refused(&message, request_id) {
                    return vec![];
                }
                let text =
                    first_line_and_more(&message).unwrap_or_else(|| format!("{request} refused"));
                if let Some(form) = self.form_waiting_on(request_id) {
                    form.error = Some(text);
                    form.submitting = false;
                    form.request_id = None;
                } else if let Some(form) = self.goal_form_waiting_on(request_id) {
                    form.error = Some(text);
                    form.submitting = false;
                    form.request_id = None;
                } else if let Some(form) = self.iterate_waiting_on(request_id) {
                    form.error = Some(text);
                    form.submitting = false;
                    form.request_id = None;
                } else {
                    self.goal_refused(request_id);
                    self.toast_at(super::ToastLevel::Error, text);
                }
            }
            RunReply::Triaged {
                run_id,
                message,
                request_id,
                ..
            } => {
                // Review: a reply to this client's goal whose form was closed meanwhile
                // is still shown. This client sends `StartGoal` only tagged, so an
                // untagged `Triaged` is not its own and changes nothing (M8c).
                if !self.goal_started(request_id, run_id, &message) && request_id.is_some() {
                    self.toast(capped(&message));
                }
            }
            // Milestone 9.10 decision 12: the goal waits for its repository's profile.
            // Until M9.10.10 the form closes as for a refused start and the daemon's
            // message is toasted.
            // Milestone 9.10 decision 35: the goal waits for its repository's profile.
            RunReply::Queued {
                message,
                request_id,
                ..
            } => {
                if !self.goal_queued(request_id, &message) && request_id.is_some() {
                    self.toast(capped(&message));
                }
            }
            // Milestone 9.3 decision 22 (D11): a continued goal's reply is `Started`.
            RunReply::Started {
                run_id, request_id, ..
            } => {
                let message = format!("run {run_id} started");
                self.goal_started(request_id, Some(run_id), &message);
            }
            RunReply::ConfirmNeeded { .. }
            | RunReply::ToolResult { .. }
            // Settings, profile and stats replies are routed by id in `app/replies.rs`
            // (decisions 24, 34 and 38); one reaching here is no request of this
            // client's.
            | RunReply::Stats { .. }
            | RunReply::Profile { .. }
            | RunReply::Settings { .. }
            // Milestone 9.6: the gate screen asks for documents from task M9.6.17.
            | RunReply::Doc { .. } => {}
            RunReply::TaskDetail { detail, request_id } => self.on_task_detail(detail, request_id),
        }
        vec![]
    }

    /// Task M8c.3: the tree follows the snapshot as it follows a window list —
    /// fold state of runs that left is pruned, the selection repaired, and the
    /// selection revealed only when the rows changed (decision 15 of milestone 4.6).
    fn replace_runs(&mut self, snapshot: RunsSnapshot) {
        let previous_keys: Vec<_> = self.nav_rows().into_iter().map(|row| row.key).collect();
        let previous_selection = self.tree.selected.clone();
        let review_at = self.review_position();
        let run_row = self
            .run_view
            .as_ref()
            .and_then(|view| tree::row_index(&self.rows(), &NodeKey::Run(view.run_id.clone())));
        self.runs = snapshot;
        self.tree.prune_runs(&self.runs.runs);
        self.tree.prune(&self.windows);
        self.close_run_view_if_gone(run_row);
        self.close_gate_modal_if_stale();
        self.follow_review(review_at);
        self.repair_alerts_focus();
        self.follow_action_flow();
        self.open_pending_run();
        self.refresh_goal_chains();
        let rows = nav_rows_of(
            &self.windows,
            &self.runs,
            &self.tree,
            self.run_view.as_ref(),
        );
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

    /// Milestone 9.5 decision 43: rate-limited, or waiting out a failed turn until its
    /// retry (the daemon clears `failed_until` once it sends the continue).
    pub fn round_waits(&self, round: &AgentRoundInfo) -> bool {
        self.rate_limited(round) || round.failed_until.is_some()
    }

    /// Decision 11: the rows the canvas, the painter, the inspector, the reveal, the
    /// selection keys and the mouse read. The sidebar keeps reading `rows()`.
    pub fn nav_rows(&self) -> Vec<Row<'_>> {
        nav_rows_of(
            &self.windows,
            &self.runs,
            &self.tree,
            self.run_view.as_ref(),
        )
    }

    /// Decision 22: the run view on `run_id`, root selected, pan at the origin, both
    /// filters reset. From the sidebar tree it turns the overview on first, through
    /// `enter_overview`, so tree navigation is on (the reversed box and the dependency
    /// highlight need it).
    pub(crate) fn open_run_view(&mut self, run_id: String) {
        if !self.overview {
            self.enter_overview();
        }
        self.tree_input = Some(TreeInput::Navigate);
        self.tree.filter.clear();
        // Review m1: an old fold of the root never hides the view.
        self.tree.collapsed.remove(&NodeKey::Run(run_id.clone()));
        self.run_view = Some(RunView {
            run_id: run_id.clone(),
            filter: RunFilter::All,
        });
        self.graph_pan = crate::graph::Pan::default();
        self.settle_graph_viewport();
        let rows = nav_rows_of(
            &self.windows,
            &self.runs,
            &self.tree,
            self.run_view.as_ref(),
        );
        self.tree.select(&rows, NodeKey::Run(run_id));
        self.reveal_tree_anchor();
    }

    /// Decision 23: back to the project overview with the run's node selected, both
    /// filters reset.
    pub(crate) fn close_run_view(&mut self) {
        self.close_run_view_at(None);
    }

    /// `close_run_view`; when the run has left the tree, the project row now at
    /// `run_row`, where its node was, clamped (review m2).
    fn close_run_view_at(&mut self, run_row: Option<usize>) {
        let Some(view) = self.run_view.take() else {
            return;
        };
        self.settle_graph_viewport();
        self.tree.filter.clear();
        if self.tree_input.is_some() {
            self.tree_input = Some(TreeInput::Navigate);
        }
        let rows = tree::build_from(&self.windows, &self.runs, &self.tree);
        let run = NodeKey::Run(view.run_id);
        let key = match (tree::row_index(&rows, &run), run_row) {
            (None, Some(at)) if !rows.is_empty() => rows[at.min(rows.len() - 1)].key.clone(),
            _ => run,
        };
        self.tree.select(&rows, key);
        self.tree.repair_selection(&rows);
        self.reveal_tree_anchor();
    }

    /// Decision 23: a snapshot that no longer names the open run, or names it in a
    /// terminal state, closes the view with a toast saying which. `run_row`: the run's
    /// project row before the snapshot.
    fn close_run_view_if_gone(&mut self, run_row: Option<usize>) {
        let Some(id) = self.run_view.as_ref().map(|view| view.run_id.clone()) else {
            return;
        };
        let text = match self.runs.runs.iter().find(|run| run.run_id == id) {
            None => format!("run {id} is gone"),
            Some(run) if run.state.is_terminal() => {
                format!("run {id} is {}", state_text(run.state))
            }
            Some(_) => return,
        };
        self.close_run_view_at(run_row);
        self.toast(text);
    }

    /// The keys only the run view has (decisions 21 and 23): `f` cycles the filter, `h`
    /// at the root and `Esc` leave. `None`: not one of them, so the project tree's rule
    /// applies (`h` below the root selects the parent). `a`, `x`, `e` and `d` are the
    /// plan gate's (decision 32) and, past the gate, an approval hold's (milestone 9);
    /// `s` submits a planning run; `p` opens the plan review (milestone 9.0.5).
    pub(crate) fn on_run_view_key(&mut self, key: KeyEvent) -> Option<Vec<Effect>> {
        let view = self.run_view.as_mut()?;
        match key.code {
            KeyCode::Char('f') => {
                view.filter = next_filter(view.filter);
                let rows = nav_rows_of(
                    &self.windows,
                    &self.runs,
                    &self.tree,
                    self.run_view.as_ref(),
                );
                self.tree.repair_selection(&rows);
                self.reveal_tree_anchor();
            }
            KeyCode::Char('h') | KeyCode::Left
                if self.tree.selected == Some(NodeKey::Run(view.run_id.clone())) =>
            {
                self.close_run_view();
            }
            KeyCode::Esc => self.close_run_view(),
            KeyCode::Char(c @ ('a' | 'x' | 'e' | 'd')) => {
                let run_id = view.run_id.clone();
                let selected = self.tree.selected.clone();
                return Some(self.on_gate_key(run_id, c, selected));
            }
            // Milestone 9.0.5 decision 25: the task panel's scroll and brief.
            KeyCode::PageUp | KeyCode::PageDown | KeyCode::Char('b') => {
                if !self.on_task_panel_key(key.code) {
                    return None;
                }
            }
            // Milestone 9.0.5 decision 14: the plan review of what awaits approval; at a
            // brainstorm or spec gate (milestone 9.6), the gate screen.
            KeyCode::Char('p') => {
                let run_id = view.run_id.clone();
                let run = self.runs.runs.iter().find(|run| run.run_id == run_id);
                if run.is_some_and(|run| super::doc_gate::doc_gate_of(run).is_some()) {
                    return Some(self.open_doc_gate(&run_id));
                }
                return Some(self.review_from_run_view(&run_id));
            }
            // Milestone 9 decision 13: the user's submit, on a planning run's root only.
            KeyCode::Char('s') => {
                let run_id = view.run_id.clone();
                if !self.on_submit_key(&run_id) {
                    return None;
                }
            }
            _ => return None,
        }
        Some(vec![])
    }

    /// Tests move the snapshot's arrival into the past instead of sleeping.
    #[cfg(test)]
    pub(crate) fn set_runs_received_at(&mut self, at: Instant) {
        self.runs_received_at = at;
    }
}
