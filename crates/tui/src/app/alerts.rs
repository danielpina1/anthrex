//! Milestone 9.0.5 decisions 17–21: the alerts, computed from the snapshot and the
//! window list on every draw and key, and the Alerts box's focus; milestone 9.0.7
//! decisions 7 and 8 give each its who, task, full detail and age. Nothing is stored
//! but the focus; an alert clears itself once what raised it is resolved. The box is
//! drawn by `ui/alerts.rs`. Pure: no I/O.

use super::{App, Effect};
use crate::actions_request::ActionTarget;
use crate::inspector::run_format::reason_text;
use crate::safe_text::one_line;
use crate::tree::{self, awaiting_holds, is_paused};
use crossterm::event::{KeyCode, KeyEvent};
use proto::{ActionKind, BlockReason, RunInfo, RunState, Runtime, Status, TaskState, WindowKind};
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

/// Who an alert is for (milestone 9.0.7 decision 7): a run, by its goal (the id when
/// it has none, as the sidebar names it) and its id, or a project, by its directory
/// name. Each already one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertWho {
    Run { goal: String, id: String },
    Project(String),
}

/// One alert (milestone 9.0.7 decision 7). `who`, `task` and `text` are already one
/// line each; `detail` is the whole text (a block's or a halted reason's every line,
/// else `text`), raw, sanitised where it is drawn. `age` is set only where the
/// snapshot holds the moment it began (decision 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub priority: u8,
    pub key: AlertKey,
    pub who: AlertWho,
    pub task: Option<String>,
    pub text: String,
    pub detail: String,
    pub age: Option<u64>,
}

/// Decision 21: the box has the keys. `selected` is the alert's identity; `at` its
/// position when last seen, so a resolved selection gives way to the alert now there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsFocus {
    pub selected: Option<AlertKey>,
    pub(crate) at: usize,
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

/// Decision 18's priority 1 for `run`, first match wins, with the orchestrator
/// window's time in its status (milestone 9.0.7 decision 8).
fn orchestrator_text(app: &App, run: &RunInfo) -> Option<(&'static str, u64)> {
    let orch = run.orchestrator.as_ref().filter(|orch| orch.live)?;
    let id = orch.window_id?;
    let window = app.windows.iter().find(|window| window.id == id)?;
    let agent = matches!(window.runtime, Runtime::Claude | Runtime::Codex);
    let quiet = matches!(
        window.status,
        Status::Idle | Status::Done | Status::Attention
    );
    let text = if orch.wake_held {
        "orchestrator wake-up held"
    } else if window.status == Status::Attention && window.signals_seen {
        "orchestrator asks for permission"
    } else if agent && !window.signals_seen && quiet {
        "orchestrator waits at a start prompt"
    } else {
        return None;
    };
    Some((text, app.elapsed_secs(window)))
}

/// Decision 18's priority 3 for a blocked task: while the orchestrator lives, only
/// what it cannot answer; with none, every reason. A paused task is never an alert,
/// nor one at the plan gate or under a hold awaiting approval: the gate's or the
/// hold's alert covers it, and it is drawn as planned (`○`, milestone 9.0.7 ruling).
fn blocked_text(run: &RunInfo, task: &proto::TaskInfo) -> Option<String> {
    if task.state != TaskState::Blocked || is_paused(task) {
        return None;
    }
    if run.state == RunState::AwaitingApproval || tree::task_held(run, task) {
        return None;
    }
    let orchestrator_lives = orchestrator_lives(run);
    let reason = task.block.as_ref().map(|block| block.reason);
    let asks_the_user = matches!(
        reason,
        Some(BlockReason::Human | BlockReason::Conflict | BlockReason::Environment)
    );
    if orchestrator_lives && !asks_the_user {
        return None;
    }
    // Milestone 9.0.7 decision 7: the task id is the alert's `task`, drawn in its who
    // line; a question reads `blocked: <line>`, any other reason names itself.
    let Some(block) = &task.block else {
        return Some("blocked".to_owned());
    };
    let head = match block.reason {
        BlockReason::Question => "blocked".to_owned(),
        reason => format!("blocked ({})", reason_text(reason)),
    };
    Some(match first_line(&block.text) {
        Some(line) => format!("{head}: {line}"),
        None => head,
    })
}

/// Decision 8: when a blocked task's block began, the newest history entry the daemon
/// wrote at a block (`blocked (<reason>): …`); `None` when the snapshot kept none.
fn blocked_since(task: &proto::TaskInfo) -> Option<u64> {
    task.history
        .iter()
        .filter(|event| event.text.starts_with("blocked ("))
        .map(|event| event.at)
        .max()
}

/// `text` when it holds a non-blank line, else `fallback`: an alert's `detail`.
fn detail_or(text: Option<&str>, fallback: &str) -> String {
    match text {
        Some(text) if first_line(text).is_some() => text.to_owned(),
        _ => fallback.to_owned(),
    }
}

/// Milestone 9.0.7 decision 4: "needs you" is one rule. A task needs the user exactly
/// when it is an alert, so its glyph (`theme::task_look`) and the alerts agree.
pub fn task_needs_you(run: &RunInfo, task: &proto::TaskInfo) -> bool {
    blocked_text(run, task).is_some()
}

fn orchestrator_lives(run: &RunInfo) -> bool {
    run.orchestrator.as_ref().is_some_and(|orch| orch.live)
}

/// Decisions 17 and 18: every alert, most urgent first — by priority, then the run's
/// `created_at` (the order `shown_runs` gives), then the order the rules list.
pub fn alerts(app: &App) -> Vec<Alert> {
    let mut out = Vec::new();
    for run in tree::shown_runs(&app.runs.runs) {
        let id = run.run_id.clone();
        let who = AlertWho::Run {
            goal: one_line(tree::run_title(run)),
            id: one_line(&id),
        };
        // `detail` defaults to the text; `age` only where decision 8 knows it.
        let mut push = |priority, key, text: String, task: Option<&str>, detail, age| {
            let text = one_line(&text);
            out.push(Alert {
                priority,
                key,
                who: who.clone(),
                task: task.map(one_line),
                detail: detail_or(detail, &text),
                text,
                age,
            });
        };
        if let Some((text, age)) = orchestrator_text(app, run) {
            let key = AlertKey::Orchestrator(id.clone());
            push(1, key, text.to_owned(), None, None, Some(age));
        }
        if run.state == RunState::AwaitingApproval {
            let n = run
                .tasks
                .iter()
                .filter(|task| task.state != TaskState::Cancelled)
                .count();
            let text = format!("plan awaits approval · {}", tasks_text(n));
            push(2, AlertKey::Gate(id.clone()), text, None, None, None);
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
            push(2, key, text, None, None, None);
        }
        for task in &run.tasks {
            if let Some(text) = blocked_text(run, task) {
                let key = AlertKey::Blocked {
                    run: id.clone(),
                    task: task.id.clone(),
                };
                let detail = task.block.as_ref().map(|block| block.text.as_str());
                let age = blocked_since(task).map(|at| app.run_age(at));
                push(3, key, text, Some(&task.id), detail, age);
            }
        }
        if run.state == RunState::Halted {
            let text = match run.halted_reason.as_deref().and_then(first_line) {
                Some(line) => format!("run halted: {line}"),
                None => "run halted".to_owned(),
            };
            let detail = run.halted_reason.as_deref();
            push(3, AlertKey::Halted(id.clone()), text, None, detail, None);
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
            let text = format!("ready to accept · {m}/{n} merged");
            push(4, AlertKey::Accept(id.clone()), text, None, None, None);
        }
    }
    for proposal in &app.runs.proposals {
        let text = "profile proposal ready".to_owned();
        out.push(Alert {
            priority: 4,
            key: AlertKey::Proposal(proposal.project.clone()),
            who: AlertWho::Project(one_line(&project_name(&proposal.project))),
            task: None,
            detail: text.clone(),
            text,
            age: Some(app.run_age(proposal.updated_at)),
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

    pub(super) fn leave_alerts(&mut self) {
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
            AlertKey::Gate(run) => self.alert_menu(run, ActionTarget::Run, ActionKind::ReviewPlan),
            AlertKey::Hold { run, hold } => {
                self.alert_menu(run, ActionTarget::Run, ActionKind::ApproveHold { hold })
            }
            AlertKey::Blocked { run, task } => {
                let question = self
                    .runs
                    .runs
                    .iter()
                    .find(|r| r.run_id == run)
                    .and_then(|r| r.tasks.iter().find(|t| t.id == task))
                    .and_then(|t| t.block.as_ref())
                    .is_some_and(|b| b.reason == BlockReason::Question);
                let kind = if question {
                    ActionKind::Answer
                } else {
                    ActionKind::Retry
                };
                self.alert_menu(run, ActionTarget::Task(task), kind)
            }
            AlertKey::Halted(run) => self.alert_menu(run, ActionTarget::Run, ActionKind::Resume),
            AlertKey::Accept(run) => self.alert_menu(run, ActionTarget::Run, ActionKind::Accept),
            // Preflight F26: the Profile screen on that project's proposal.
            AlertKey::Proposal(project) => self.open_profile_on(project, true),
        }
    }

    /// Decision 17: Enter on an alert opens the menu on its node with `kind` selected
    /// (the first entry when the state moved and `kind` is no longer listed).
    fn alert_menu(&mut self, run: String, target: ActionTarget, kind: ActionKind) -> Vec<Effect> {
        self.open_actions((run, target), Some(kind))
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
