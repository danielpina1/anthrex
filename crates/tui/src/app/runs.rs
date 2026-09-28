//! Milestone 8c: the client's copy of the run snapshot (task M8c.2, decisions 1, 2 and
//! 34). One pushed `RunsSnapshot`, subscribed once per connection, replaced whole by
//! every push; the daemon's clock as the client sees it; and the toasts for the run
//! replies this client asked for. Task M8c.6 adds the run view's state (decision 11):
//! opening, leaving and the keys only it has; Enter inside it is `run_enter.rs`. Task
//! M8c.9 adds the plan gate (decisions 32–34): `a`, `x`, `e` and `d`, the edit form's
//! keys and its replies. The client only ever sends the user's own requests, each after
//! a confirm (the `y` of a `Confirm`, or the form's `Enter`).

use super::{App, Effect, Modal, PendingAction, TreeInput};
use crate::run_edit::{EditOutcome, TaskEditForm};
use crate::tree::{self, NodeKey, Row, RunFilter, TreeState};
use crossterm::event::{KeyCode, KeyEvent};
use proto::run_wire::request::EDIT;
use proto::{
    AgentRoundInfo, ClientMsg, RunInfo, RunPath, RunReply, RunRequest, RunState, RunsSnapshot,
    Runtime, TaskInfo, TaskState, WindowInfo,
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
    runs: &'a [RunInfo],
    state: &TreeState,
    view: Option<&RunView>,
) -> Vec<Row<'a>> {
    let shown = view.and_then(|view| {
        tree::shown_runs(runs)
            .find(|run| run.run_id == view.run_id)
            .map(|run| (run, view.filter))
    });
    match shown {
        Some((run, filter)) => tree::run_rows(run, windows, state, filter),
        None => tree::build_with_runs(windows, runs, state),
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
    }
}

fn confirm(message: String, action: PendingAction) -> Modal {
    Modal::Confirm { message, action }
}

/// Decision 32's approve confirm: the tasks not `cancelled` are the ones that start.
fn approve_message(run: &RunInfo) -> String {
    let run_id = &run.run_id;
    let starting = run.tasks.iter().filter(|t| t.state != TaskState::Cancelled);
    match starting.count() {
        1 => format!("Approve run {run_id}? 1 task starts."),
        n => format!("Approve run {run_id}? {n} tasks start."),
    }
}

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
            // Decision 34: an edit's reply ends a submitting form — closed on `Done`,
            // its error row filled on `Refused`. Every other reply is a toast.
            RunReply::Done {
                request, message, ..
            } => {
                if request == EDIT && self.submitting_form().is_some() {
                    self.modal = None;
                }
                self.toast(capped(&message));
            }
            RunReply::Refused {
                request, message, ..
            } => {
                let text =
                    first_line_and_more(&message).unwrap_or_else(|| format!("{request} refused"));
                match self.submitting_form().filter(|_| request == EDIT) {
                    Some(form) => {
                        form.error = Some(text);
                        form.submitting = false;
                    }
                    None => self.toast(text),
                }
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
        let previous_keys: Vec<_> = self.nav_rows().into_iter().map(|row| row.key).collect();
        let previous_selection = self.tree.selected.clone();
        let run_row = self
            .run_view
            .as_ref()
            .and_then(|view| tree::row_index(&self.rows(), &NodeKey::Run(view.run_id.clone())));
        self.runs = snapshot;
        self.tree.prune_runs(&self.runs.runs);
        self.tree.prune(&self.windows);
        self.close_run_view_if_gone(run_row);
        self.close_gate_modal_if_stale();
        let rows = nav_rows_of(
            &self.windows,
            &self.runs.runs,
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

    /// Decision 11: the rows the canvas, the painter, the inspector, the reveal, the
    /// selection keys and the mouse read. The sidebar keeps reading `rows()`.
    pub fn nav_rows(&self) -> Vec<Row<'_>> {
        nav_rows_of(
            &self.windows,
            &self.runs.runs,
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
            &self.runs.runs,
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
        let rows = tree::build_with_runs(&self.windows, &self.runs.runs, &self.tree);
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
    /// plan gate's (decision 32).
    pub(crate) fn on_run_view_key(&mut self, key: KeyEvent) -> Option<Vec<Effect>> {
        let view = self.run_view.as_mut()?;
        match key.code {
            KeyCode::Char('f') => {
                view.filter = next_filter(view.filter);
                let rows = nav_rows_of(
                    &self.windows,
                    &self.runs.runs,
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
                self.on_gate_key(run_id, c);
            }
            _ => return None,
        }
        Some(vec![])
    }

    /// Decision 32: `Ok` with the run while it awaits approval — the gate is open —
    /// else the toast saying why it is closed.
    fn gate_run(&self, run_id: &str) -> Result<&RunInfo, String> {
        let closed = |why: &str| format!("the plan gate is closed: run {run_id} is {why}");
        match self.runs.runs.iter().find(|run| run.run_id == run_id) {
            Some(run) if run.state == RunState::AwaitingApproval => Ok(run),
            Some(run) if run.path == Some(RunPath::Fast) => Err(closed("on the fast path")),
            Some(run) => Err(closed(state_text(run.state))),
            None => Err(closed("gone")),
        }
    }

    /// Decision 32's four keys: `a` and `x` ask to approve or reject the run, `d` to
    /// remove the selected task, `e` opens the edit form on it. Nothing is sent here.
    fn on_gate_key(&mut self, run_id: String, key: char) {
        let run = match self.gate_run(&run_id) {
            Ok(run) => run,
            Err(text) => return self.toast(text),
        };
        let task = match &self.tree.selected {
            Some(NodeKey::Task { run: r, id }) if *r == run_id => {
                run.tasks.iter().find(|task| task.id == *id)
            }
            _ => None,
        };
        let modal = match (key, task) {
            ('a', _) => confirm(approve_message(run), PendingAction::ApproveRun(run_id)),
            ('x', _) => confirm(
                format!(
                    "Reject run {run_id}? Its branches and worktrees are removed; \
                     salvage refs are kept."
                ),
                PendingAction::RejectRun(run_id),
            ),
            ('e', Some(task)) => Modal::EditTask(TaskEditForm::new(&run_id, task)),
            (_, Some(task)) => confirm(
                format!("Remove {} from run {run_id}'s plan?", task.id),
                PendingAction::RemoveTask {
                    task_id: task.id.clone(),
                    run_id,
                },
            ),
            (_, None) => return self.toast("select a task to edit or remove"),
        };
        self.modal = Some(modal);
    }

    /// Decision 33: the open edit form's keys. `Enter` sends the one `Edit` and leaves
    /// the form open, submitting, until its reply (decision 34).
    pub(crate) fn on_edit_task_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(Modal::EditTask(form)) = &mut self.modal else {
            return vec![];
        };
        match form.on_key(key) {
            EditOutcome::Stay => vec![],
            EditOutcome::Cancel => {
                self.modal = None;
                vec![]
            }
            EditOutcome::Unchanged => {
                self.modal = None;
                self.toast("nothing changed");
                vec![]
            }
            EditOutcome::Submit(edits) => {
                let run_id = form.run_id.clone();
                vec![Effect::Send(ClientMsg::Run(RunRequest::Edit {
                    run_id,
                    edits,
                }))]
            }
        }
    }

    /// The edit form while it waits for its `run edit` reply.
    fn submitting_form(&mut self) -> Option<&mut TaskEditForm> {
        match &mut self.modal {
            Some(Modal::EditTask(form)) if form.submitting => Some(form),
            _ => None,
        }
    }

    /// Whole-branch review M2: the `Edit` a submitting form waits on was refused by the
    /// connection, or its reply went with a lost link, so no reply will come. The form
    /// stops submitting and says so inline; `Enter` sends it again.
    pub(super) fn edit_not_sent(&mut self) {
        if let Some(form) = self.submitting_form() {
            form.submitting = false;
            form.error = Some("the edit was not sent; press Enter to retry".into());
        }
    }

    /// Decision 32's task while the gate is open, else the toast saying why not.
    fn gate_task(&self, run_id: &str, task_id: &str) -> Result<&TaskInfo, String> {
        let gone = || format!("{task_id} is no longer in run {run_id}'s plan");
        let run = self.gate_run(run_id)?;
        run.tasks.iter().find(|t| t.id == task_id).ok_or_else(gone)
    }

    /// After every snapshot (review I1 and M6): an open gate `Confirm` or edit form
    /// whose request could now only be refused, or would do other than it said, closes
    /// with a toast saying why. A `y` or `Enter` after it sends nothing.
    fn close_gate_modal_if_stale(&mut self) {
        let text = match &self.modal {
            Some(Modal::EditTask(form)) => self.stale_form(form),
            Some(Modal::Confirm { message, action }) => self.stale_confirm(message, action),
            _ => None,
        };
        if let Some(text) = text {
            self.modal = None;
            self.toast(text);
        }
    }

    /// The gate closed, the task gone, or — while not submitting, since our own edit
    /// changes these — the task's values changed elsewhere, so its route would revert.
    fn stale_form(&self, form: &TaskEditForm) -> Option<String> {
        let (run_id, task_id) = (&form.run_id, &form.task_id);
        match self.gate_task(run_id, task_id) {
            Err(text) => Some(text),
            Ok(task) if !form.submitting && !form.opened_from(task) => {
                Some(format!("{task_id} changed in run {run_id}; press e again"))
            }
            Ok(_) => None,
        }
    }

    /// The gate closed; for a remove, its task gone or finished; for an approve, a
    /// different number of tasks would start than the confirm says.
    fn stale_confirm(&self, message: &str, action: &PendingAction) -> Option<String> {
        match action {
            PendingAction::ApproveRun(run_id) => match self.gate_run(run_id) {
                Err(text) => Some(text),
                Ok(run) if approve_message(run) != message => {
                    Some(format!("run {run_id}'s plan changed; press a again"))
                }
                Ok(_) => None,
            },
            PendingAction::RejectRun(run_id) => self.gate_run(run_id).err(),
            PendingAction::RemoveTask { run_id, task_id } => {
                match self.gate_task(run_id, task_id) {
                    Err(text) => Some(text),
                    Ok(task) if task.state.is_finished() => {
                        Some(format!("{task_id} is no longer in run {run_id}'s plan"))
                    }
                    Ok(_) => None,
                }
            }
            _ => None,
        }
    }

    /// Tests move the snapshot's arrival into the past instead of sleeping.
    #[cfg(test)]
    pub(crate) fn set_runs_received_at(&mut self, at: Instant) {
        self.runs_received_at = at;
    }
}
