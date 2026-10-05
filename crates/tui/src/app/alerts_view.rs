//! Milestone 9.0.7 decision 11: the Alerts view's keys, and what its rows and hints
//! read off an alert — the node its menu opens on, the entries that node lists, and
//! the entry Enter preselects (9.0.6 decision 17). `ui/alerts_view.rs` draws the view.
//! Pure: no I/O.

use super::alerts::{AlertKey, StageAlert, alerts};
use super::{App, Effect};
use crate::actions_request::ActionTarget;
use crate::keymap::Command;
use crate::tree::NodeKey;
use crossterm::event::{KeyCode, KeyEvent};
use proto::{ActionInfo, ActionKind, BlockReason, RunInfo, WindowKind};

/// The node an alert's menu opens on: its run, or its blocked task; none for a
/// proposal, which has no run. An orchestrator alert's node is its run.
pub(crate) fn alert_node(key: &AlertKey) -> Option<(String, ActionTarget)> {
    match key {
        AlertKey::Orchestrator(run)
        | AlertKey::Gate(run)
        | AlertKey::Halted(run)
        | AlertKey::Accept(run)
        | AlertKey::Hold { run, .. }
        | AlertKey::Delivery { run, .. } => Some((run.clone(), ActionTarget::Run)),
        AlertKey::Blocked { run, task } => Some((run.clone(), ActionTarget::Task(task.clone()))),
        // Milestone 9.5 decision 45: a held stage resumes from its run's menu.
        AlertKey::Stage {
            run,
            kind: StageAlert::Held,
            ..
        } => Some((run.clone(), ActionTarget::Run)),
        AlertKey::Stage { run, stage, .. } => Some((run.clone(), ActionTarget::Stage(*stage))),
        AlertKey::Proposal(_) => None,
    }
}

/// Fix round 1 ruling: the toast for a command refused under the view.
pub const LEAVE_ALERTS_FIRST: &str = "leave the alerts first (esc)";

pub(crate) fn run_of<'a>(app: &'a App, run_id: &str) -> Option<&'a RunInfo> {
    app.runs.runs.iter().find(|run| run.run_id == run_id)
}

/// The node's menu entries, in the menu's own order (`actions::menu_items`).
fn menu(app: &App, key: &AlertKey) -> Vec<ActionInfo> {
    alert_node(key)
        .and_then(|(run, target)| super::actions::menu_items(run_of(app, &run)?, &target))
        .unwrap_or_default()
}

/// The view's `actions` row (decision 11): the node's own entries, the daemon's first,
/// then the client's (`local_actions`, 9.0.6 decision 10).
pub(crate) fn alert_actions(app: &App, key: &AlertKey) -> Vec<ActionInfo> {
    let (local, daemon): (Vec<_>, Vec<_>) = menu(app, key)
        .into_iter()
        .partition(|action| action.kind.is_local());
    daemon.into_iter().chain(local).collect()
}

/// The kind Enter selects in the menu, by priority (9.0.5 decision 18, 9.0.6 decision
/// 17): none where Enter opens no menu (an orchestrator, a proposal).
pub(crate) fn preselected(app: &App, key: &AlertKey) -> Option<ActionKind> {
    match key {
        AlertKey::Orchestrator(_) | AlertKey::Proposal(_) => None,
        // Milestone 9.6 decision 34: a brainstorm or spec gate's document is reviewed on
        // the gate screen; a plan gate (a design run's too) on the plan review.
        AlertKey::Gate(run) => Some(
            match run_of(app, run).and_then(super::doc_gate::doc_gate_of) {
                Some(_) => ActionKind::ReviewDoc,
                None => ActionKind::ReviewPlan,
            },
        ),
        AlertKey::Hold { hold, .. } => Some(ActionKind::ApproveHold { hold: hold.clone() }),
        AlertKey::Blocked { run, task } => {
            let question = run_of(app, run)
                .and_then(|r| r.tasks.iter().find(|t| t.id == *task))
                .and_then(|t| t.block.as_ref())
                .is_some_and(|b| b.reason == BlockReason::Question);
            Some(if question {
                ActionKind::Answer
            } else {
                ActionKind::Retry
            })
        }
        AlertKey::Halted(_) => Some(ActionKind::Resume),
        AlertKey::Accept(_) => Some(ActionKind::Accept),
        // Ruling R-13: a held stage or op resumes (`run resume` pushes it again); the
        // rest are the user's to do on GitHub or with `gh`, so the menu's first entry.
        AlertKey::Delivery { kind, .. } => {
            (*kind == proto::DeliveryAlertKind::HostOpHeld).then_some(ActionKind::Resume)
        }
        // Decision 45: `anthrex run resume` retries a held stage's tier 3.
        AlertKey::Stage { kind, .. } => (*kind == StageAlert::Held).then_some(ActionKind::Resume),
    }
}

/// What Enter's hint names: the entry the menu selects (the preselection where listed,
/// else the first), `focus` for an orchestrator, `open profile` for a proposal. Raw:
/// the hint line sanitises it.
pub(crate) fn enter_label(app: &App, key: &AlertKey) -> String {
    match key {
        AlertKey::Orchestrator(_) => "focus".to_owned(),
        AlertKey::Proposal(_) => "open profile".to_owned(),
        _ => {
            let items = menu(app, key);
            let at = preselected(app, key)
                .and_then(|kind| items.iter().position(|a| a.kind == kind))
                .unwrap_or(0);
            items
                .get(at)
                .map_or_else(|| "actions".to_owned(), |a| a.label.clone())
        }
    }
}

/// `m` is offered: a task alert whose node lists `Message`, not refused.
pub(crate) fn can_message(app: &App, key: &AlertKey) -> bool {
    matches!(key, AlertKey::Blocked { .. })
        && menu(app, key)
            .iter()
            .any(|a| a.kind == ActionKind::Message && a.refused_why.is_none())
}

impl App {
    /// Fix round 1 ruling: no command acts unseen under the view. While it is open,
    /// whatever changes or acts on what lies under the main pane — the focused window
    /// (`C-b j`/`k`/`1-9`, `c`, `x`, `X`, `,`, `R`, `g`), the conversation, the tree and
    /// the overview (`m`, `t`, `T`) — is refused with a toast, as `screen_refuses`
    /// does. The help, detach, stop, the sidebar's keys, reconnect and the full-body
    /// screens (which cover the view, and give it back) keep working.
    pub(super) fn alerts_refuses(&mut self, cmd: Command) -> bool {
        if self.alerts_focus.is_none() {
            return false;
        }
        let refused = matches!(
            cmd,
            Command::NextWindow
                | Command::PrevWindow
                | Command::FocusIndex(_)
                | Command::NewWindow
                | Command::KillWindow
                | Command::RemoveWindow
                | Command::RenameWindow
                | Command::RestartWindow
                | Command::StartGoal
                | Command::ToggleConversation
                | Command::ToggleTree
                | Command::ToggleOverview
        );
        if refused {
            self.toast(LEAVE_ALERTS_FIRST);
        }
        refused
    }

    /// Decision 11's keys: `j`/`k` move, Enter runs the preselection, `.` the menu with
    /// none, `m` the menu on `Message` where listed, `o` the run view (each leaving the
    /// view), PgUp/PgDn scroll the detail, Esc leaves. Every other key does nothing;
    /// none sends input.
    pub(super) fn alerts_view_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let selected = self.alerts_focus.as_ref().and_then(|f| f.selected.clone());
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_alert(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_alert(-1),
            KeyCode::PageDown => self.scroll_alert_detail(true),
            KeyCode::PageUp => self.scroll_alert_detail(false),
            KeyCode::Esc => self.leave_alerts(),
            // Final fix wave M2: with no alert selected (an empty view) Enter does
            // nothing; Esc alone leaves (decision 11).
            KeyCode::Enter => {
                if let Some(key) = selected {
                    self.leave_alerts();
                    return self.enter_alert(key);
                }
            }
            KeyCode::Char('.') => {
                if let Some(node) = selected.as_ref().and_then(alert_node) {
                    self.leave_alerts();
                    return self.open_actions(node, None);
                }
            }
            KeyCode::Char('m') => {
                let node = selected
                    .filter(|key| can_message(self, key))
                    .as_ref()
                    .and_then(alert_node);
                if let Some(node) = node {
                    self.leave_alerts();
                    return self.open_actions(node, Some(ActionKind::Message));
                }
            }
            KeyCode::Char('o') => {
                if let Some(key) = selected {
                    self.leave_alerts();
                    return self.open_alert_node(key);
                }
            }
            _ => {}
        }
        vec![]
    }

    fn move_alert(&mut self, delta: isize) {
        let keys: Vec<AlertKey> = alerts(self).into_iter().map(|alert| alert.key).collect();
        let Some(focus) = self.alerts_focus.as_mut() else {
            return;
        };
        if keys.is_empty() {
            return;
        }
        let at = focus.at.saturating_add_signed(delta).min(keys.len() - 1);
        focus.at = at;
        focus.selected = Some(keys[at].clone());
        focus.scroll = 0;
    }

    /// PgUp/PgDn: the detail by a page, clamped to the last first row, both by the
    /// renderer's own count at the last frame's main area (`ui::alerts_view`).
    fn scroll_alert_detail(&mut self, down: bool) {
        let Some(main) = self.graph_main else {
            return;
        };
        let Some((max, page)) = crate::ui::alerts_view::detail_scroll(self, main) else {
            return;
        };
        let Some(focus) = self.alerts_focus.as_mut() else {
            return;
        };
        let from = focus.scroll.min(max);
        focus.scroll = if down {
            from.saturating_add(page).min(max)
        } else {
            from.saturating_sub(page)
        };
    }

    /// Decision 18's Enter, by priority: the orchestrator's window, the Profile screen
    /// on a proposal, else the menu on the alert's node with its preselection.
    pub(super) fn enter_alert(&mut self, key: AlertKey) -> Vec<Effect> {
        match &key {
            AlertKey::Orchestrator(run_id) => self.enter_orchestrator(run_id),
            // Preflight F26: the Profile screen on that project's proposal.
            AlertKey::Proposal(project) => self.open_profile_on(project.clone(), true),
            _ => match alert_node(&key) {
                Some(node) => {
                    let kind = preselected(self, &key);
                    self.open_actions(node, kind)
                }
                None => vec![],
            },
        }
    }

    /// Priority 1's Enter: focus the orchestrator's window and leave tree mode, as
    /// `enter_run_root` does; a headless one opens its conversation.
    fn enter_orchestrator(&mut self, run_id: &str) -> Vec<Effect> {
        let window = run_of(self, run_id)
            .and_then(|run| run.orchestrator.as_ref()?.window_id)
            .and_then(|id| self.windows.iter().find(|w| w.id == id))
            .map(|w| (w.id, w.kind));
        match window {
            None => vec![],
            Some((id, WindowKind::Headless)) => self.open_conversation(id),
            Some((id, WindowKind::Pty)) => {
                let effects = self.focus(id);
                self.exit_tree();
                effects
            }
        }
    }

    /// `o`: the run view on the alert's run with its task selected (the root for a run
    /// alert); a proposal opens the Profile screen, as Enter. An open conversation
    /// closes, so the run view is what the main pane shows.
    pub(crate) fn open_alert_node(&mut self, key: AlertKey) -> Vec<Effect> {
        let (run, task) = match key {
            AlertKey::Proposal(project) => return self.open_profile_on(project, true),
            AlertKey::Blocked { run, task } => (run, Some(task)),
            other => match alert_node(&other) {
                Some((run, _)) => (run, None),
                None => return vec![],
            },
        };
        if run_of(self, &run).is_none() {
            return vec![];
        }
        let effects = if self.conversation.is_open() {
            self.toggle_conversation()
        } else {
            Vec::new()
        };
        self.open_run_view(run.clone());
        if let Some(id) = task {
            let rows = super::nav_rows_of(
                &self.windows,
                &self.runs,
                &self.tree,
                self.run_view.as_ref(),
            );
            self.tree.select(&rows, NodeKey::Task { run, id });
            self.reveal_tree_anchor();
        }
        effects
    }
}
