//! Milestone 9.0.5 decisions 17–21: the alerts, computed from the snapshot and the
//! window list on every draw and key, and the Alerts box's focus. Nothing is stored
//! but the focus; an alert clears itself once what raised it is resolved. The box is
//! drawn by `ui/alerts.rs`. Pure: no I/O.

use super::plan_review::ReviewTarget;
use super::{App, Effect};
use crate::inspector::run_format::reason_text;
use crate::safe_text::one_line;
use crate::tree::{self, NodeKey, awaiting_holds, is_paused};
use crossterm::event::{KeyCode, KeyEvent};
use proto::{BlockReason, RunInfo, RunState, Runtime, Status, TaskState, WindowKind};
use std::path::PathBuf;

/// What an alert is about: its identity, which the focus follows (decision 21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertKey {
    Orchestrator(String),
    Gate(String),
    Hold { run: String, hold: String },
    Blocked { run: String, task: String },
    Halted(String),
    Accept(String),
    Proposal(PathBuf),
}

/// One line of the box: `● <label>  <text>` (decision 19). `label` and `text` are
/// already one line each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub priority: u8,
    pub key: AlertKey,
    pub label: String,
    pub text: String,
}

/// Decision 21: the box has the keys. `selected` is the alert's identity; `at` its
/// position when last seen, so a resolved selection gives way to the alert now there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsFocus {
    pub selected: Option<AlertKey>,
    pub(crate) at: usize,
}

/// The toast of a proposal's Enter (Interfaces "Exact user-visible text").
fn proposal_toast(project: &str) -> String {
    format!(
        "profile proposal for {project}: run anthrex profile show --proposed, then \
         anthrex profile confirm or reject, in that project"
    )
}

/// `1 task` or `<n> tasks`.
fn tasks_text(n: usize) -> String {
    match n {
        1 => "1 task".to_owned(),
        n => format!("{n} tasks"),
    }
}

/// The first non-blank line of `text`, one line and trimmed.
fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| one_line(line).trim().to_owned())
        .find(|line| !line.is_empty())
}

/// Decision 18's priority 1 for `run`, first match wins.
fn orchestrator_text(app: &App, run: &RunInfo) -> Option<&'static str> {
    let orch = run.orchestrator.as_ref().filter(|orch| orch.live)?;
    let id = orch.window_id?;
    let window = app.windows.iter().find(|window| window.id == id)?;
    let agent = matches!(window.runtime, Runtime::Claude | Runtime::Codex);
    let quiet = matches!(
        window.status,
        Status::Idle | Status::Done | Status::Attention
    );
    if orch.wake_held {
        Some("orchestrator wake-up held")
    } else if window.status == Status::Attention && window.signals_seen {
        Some("orchestrator asks for permission")
    } else if agent && !window.signals_seen && quiet {
        Some("orchestrator waits at a start prompt")
    } else {
        None
    }
}

/// Decision 18's priority 3 for a blocked task: while the orchestrator lives, only
/// what it cannot answer; with none, every reason. A paused task is never an alert.
fn blocked_text(task: &proto::TaskInfo, orchestrator_lives: bool) -> Option<String> {
    if task.state != TaskState::Blocked || is_paused(task) {
        return None;
    }
    let reason = task.block.as_ref().map(|block| block.reason);
    let asks_the_user = matches!(
        reason,
        Some(BlockReason::Human | BlockReason::Conflict | BlockReason::Environment)
    );
    if orchestrator_lives && !asks_the_user {
        return None;
    }
    let id = one_line(&task.id);
    let Some(block) = &task.block else {
        return Some(format!("{id} blocked"));
    };
    let head = format!("{id} blocked ({})", reason_text(block.reason));
    Some(match first_line(&block.text) {
        Some(line) => format!("{head}: {line}"),
        None => head,
    })
}

/// Decisions 17 and 18: every alert, most urgent first — by priority, then the run's
/// `created_at` (the order `shown_runs` gives), then the order the rules list.
pub fn alerts(app: &App) -> Vec<Alert> {
    let mut out = Vec::new();
    for run in tree::shown_runs(&app.runs.runs) {
        let id = run.run_id.clone();
        let mut push = |priority, key, text: String| {
            out.push(Alert {
                priority,
                key,
                label: one_line(&id),
                text: one_line(&text),
            });
        };
        if let Some(text) = orchestrator_text(app, run) {
            push(1, AlertKey::Orchestrator(id.clone()), text.to_owned());
        }
        if run.state == RunState::AwaitingApproval {
            let n = run
                .tasks
                .iter()
                .filter(|task| task.state != TaskState::Cancelled)
                .count();
            let text = format!("plan awaits approval · {}", tasks_text(n));
            push(2, AlertKey::Gate(id.clone()), text);
        }
        for hold in awaiting_holds(run) {
            let text = format!(
                "hold {} awaits approval · {}",
                one_line(&hold.id),
                tasks_text(hold.tasks.len())
            );
            let key = AlertKey::Hold {
                run: id.clone(),
                hold: hold.id.clone(),
            };
            push(2, key, text);
        }
        let lives = run.orchestrator.as_ref().is_some_and(|orch| orch.live);
        for task in &run.tasks {
            if let Some(text) = blocked_text(task, lives) {
                let key = AlertKey::Blocked {
                    run: id.clone(),
                    task: task.id.clone(),
                };
                push(3, key, text);
            }
        }
        if run.state == RunState::Halted {
            let text = match run.halted_reason.as_deref().and_then(first_line) {
                Some(line) => format!("run halted: {line}"),
                None => "run halted".to_owned(),
            };
            push(3, AlertKey::Halted(id.clone()), text);
        }
        if run.state == RunState::Complete {
            let counted = run
                .tasks
                .iter()
                .filter(|task| task.state != TaskState::Cancelled);
            let n = counted.clone().count();
            let m = counted
                .filter(|task| task.state == TaskState::Merged)
                .count();
            push(
                4,
                AlertKey::Accept(id.clone()),
                format!("ready to accept · {m}/{n} merged"),
            );
        }
    }
    for proposal in &app.runs.proposals {
        out.push(Alert {
            priority: 4,
            key: AlertKey::Proposal(proposal.project.clone()),
            label: one_line(&project_name(&proposal.project)),
            text: "profile proposal ready".to_owned(),
        });
    }
    // Stable: within a priority the runs' order, then the rules', then the proposals.
    out.sort_by_key(|alert| alert.priority);
    out
}

/// The project's directory name, or the whole path when it has none.
fn project_name(project: &std::path::Path) -> String {
    project.file_name().map_or_else(
        || project.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

impl App {
    /// `C-b a` (decision 21): shows a hidden sidebar and gives the box the keys, its
    /// first alert selected. Refused while the plan review is open (decision 13).
    pub(super) fn focus_alerts(&mut self) -> Vec<Effect> {
        if self.plan_review.is_some() {
            self.toast(super::plan_review::LEAVE_REVIEW_FIRST);
            return vec![];
        }
        self.sidebar_visible = true;
        let selected = alerts(self).into_iter().next().map(|alert| alert.key);
        self.alerts_focus = Some(AlertsFocus { selected, at: 0 });
        self.sync_alerts_mode();
        vec![]
    }

    /// Risks 1: the keymap's alerts mode follows `alerts_focus`, here only.
    fn sync_alerts_mode(&mut self) {
        self.keymap.set_alerts_mode(self.alerts_focus.is_some());
    }

    fn leave_alerts(&mut self) {
        self.alerts_focus = None;
        self.sync_alerts_mode();
    }

    /// Decision 21's keys: `j`/`k` move, Enter jumps (and leaves the focus), `Esc`
    /// leaves. Every other key does nothing.
    pub(super) fn on_alerts_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        self.repair_alerts_focus();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_alert(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_alert(-1),
            KeyCode::Esc => self.leave_alerts(),
            KeyCode::Enter => {
                let selected = self.alerts_focus.as_ref().and_then(|f| f.selected.clone());
                self.leave_alerts();
                if let Some(key) = selected {
                    return self.enter_alert(key);
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
    }

    /// Decision 18's Enter, by priority.
    fn enter_alert(&mut self, key: AlertKey) -> Vec<Effect> {
        match key {
            AlertKey::Orchestrator(run_id) => self.enter_orchestrator(&run_id),
            AlertKey::Gate(run_id) => self.open_plan_review(run_id, ReviewTarget::Gate),
            AlertKey::Hold { run, hold } => self.open_plan_review(run, ReviewTarget::Hold(hold)),
            AlertKey::Blocked { run, task } => {
                self.open_run_view(run.clone());
                self.select_in_run_view(NodeKey::Task { run, id: task });
                vec![]
            }
            AlertKey::Halted(run_id) | AlertKey::Accept(run_id) => {
                self.open_run_view(run_id);
                vec![]
            }
            AlertKey::Proposal(project) => {
                self.toast(proposal_toast(&project_name(&project)));
                vec![]
            }
        }
    }

    /// Selects `key` in the run view's rows and reveals it.
    fn select_in_run_view(&mut self, key: NodeKey) {
        let rows = super::nav_rows_of(
            &self.windows,
            &self.runs.runs,
            &self.tree,
            self.run_view.as_ref(),
        );
        self.tree.select(&rows, key);
        self.reveal_tree_anchor();
    }

    /// Priority 1's Enter: focus the orchestrator's window and leave tree mode, as
    /// `enter_run_root` does; a headless one opens its conversation.
    fn enter_orchestrator(&mut self, run_id: &str) -> Vec<Effect> {
        let window = self
            .runs
            .runs
            .iter()
            .find(|run| run.run_id == run_id)
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

    /// After every snapshot and window list, and before a key: the selection's
    /// position follows its alert; a selection that resolved gives way to the alert
    /// now at its old position, clamped, or to none.
    pub(super) fn repair_alerts_focus(&mut self) {
        if self.alerts_focus.is_none() {
            return;
        }
        let keys: Vec<AlertKey> = alerts(self).into_iter().map(|alert| alert.key).collect();
        let Some(focus) = self.alerts_focus.as_mut() else {
            return;
        };
        let found = focus
            .selected
            .as_ref()
            .and_then(|key| keys.iter().position(|other| other == key));
        match found {
            Some(at) => focus.at = at,
            None if keys.is_empty() => {
                focus.selected = None;
                focus.at = 0;
            }
            None => {
                focus.at = focus.at.min(keys.len() - 1);
                focus.selected = Some(keys[focus.at].clone());
            }
        }
    }
}
