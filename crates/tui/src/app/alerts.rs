//! Milestone 9.0.5 decisions 17–21: the alerts, computed from the snapshot and the
//! window list on every draw and key, and the Alerts box's focus; milestone 9.0.7
//! decisions 7 and 8 give each its who, task, full detail and age. Nothing is stored
//! but the Alerts view's state; an alert clears itself once what raised it is
//! resolved. The box is drawn by `ui/alerts.rs`, the view by `ui/alerts_view.rs`, and
//! the view's keys are `app/alerts_view.rs`'s (decision 11). Pure: no I/O.

use super::alerts_route::{Route, RouteKind, alert_route, is_you, orchestrator_lives, route_kind};
pub use super::alerts_stage::StageAlert;
use super::alerts_stage::stage_alerts;
use super::{App, Effect, ReviewTarget};
use crate::inspector::run_format::reason_text;
use crate::safe_text::one_line;
use crate::tree::{self, awaiting_holds, is_paused};
use crossterm::event::KeyEvent;
use proto::{
    BlockReason, DeliveryAlertKind, OrchestratorStuck, RunInfo, RunState, Runtime, Status,
    TaskState,
};
use std::path::PathBuf;

/// What an alert is about: its identity, which the focus follows (decision 21).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertKey {
    Orchestrator(String),
    /// Milestone 9.9 decision 24: the orchestrator's pending `ask_user`.
    OrchestratorAsks(String),
    /// Milestone 9.9 decision 21: the orchestrator is stalled or dead.
    OrchestratorStuck(String),
    Gate(String),
    Hold {
        run: String,
        hold: String,
    },
    Blocked {
        run: String,
        task: String,
    },
    Halted(String),
    Accept(String),
    Proposal(PathBuf),
    /// Milestone 9.2 ruling R-13: one of a `pr` run's typed delivery alerts
    /// (`DeliveryInfo.alerts`), the `n`-th of its kind and stage.
    Delivery {
        run: String,
        kind: DeliveryAlertKind,
        stage: Option<u16>,
        n: usize,
    },
    /// Milestone 9.5 decision 45: one of a stage's tier-3 states.
    Stage {
        run: String,
        stage: u16,
        kind: StageAlert,
    },
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
    /// Milestone 9.9 decision 23: the user's only because the cause is user-only, while
    /// the orchestrator lives; drawn as a `you` badge.
    pub you: bool,
}

/// The Alerts view's state (milestone 9.0.7 decision 11; 9.0.5 decision 21's focus).
/// `selected` is the alert's identity; `at` its position when last seen, so a resolved
/// selection gives way to the alert now there; `scroll` the detail's first row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsFocus {
    pub selected: Option<AlertKey>,
    pub(crate) at: usize,
    pub scroll: u16,
}

/// `1 task` or `<n> tasks`.
pub(crate) fn tasks_text(n: usize) -> String {
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

/// Decision 18's priority 1 for `run`, first match wins, with its age (milestone
/// 9.0.7 decision 8): the orchestrator window's time in its status where that status
/// is the wait (asking for permission, quiet at a start prompt); none for a held
/// wake-up, which the window's status time does not date (fix round 1 ruling).
fn orchestrator_text(app: &App, run: &RunInfo) -> Option<(&'static str, Option<u64>)> {
    let orch = run.orchestrator.as_ref().filter(|orch| orch.live)?;
    let id = orch.window_id?;
    let window = app.windows.iter().find(|window| window.id == id)?;
    let agent = matches!(window.runtime, Runtime::Claude | Runtime::Codex);
    let quiet = matches!(
        window.status,
        Status::Idle | Status::Done | Status::Attention
    );
    let age = Some(app.elapsed_secs(window));
    if orch.wake_held {
        Some(("orchestrator wake-up held", None))
    } else if window.status == Status::Attention && window.signals_seen {
        Some(("orchestrator asks for permission", age))
    } else if agent && !window.signals_seen && quiet {
        Some(("orchestrator waits at a start prompt", age))
    } else {
        None
    }
}

/// Milestone 9.9 decision 21: the stuck orchestrator's alert, with how long ago it began
/// when the daemon knows.
fn stuck_text(app: &App, run: &RunInfo) -> Option<(String, Option<u64>)> {
    let orch = run.orchestrator.as_ref()?;
    Some(match orch.stuck? {
        OrchestratorStuck::Stalled { since } => {
            let age = app.run_age(since);
            let text = format!(
                "orchestrator has not acted for {} min; its alerts are yours",
                age / 60
            );
            (text, Some(age))
        }
        OrchestratorStuck::Dead { since } => {
            let name = orch
                .window_id
                .and_then(|id| app.windows.iter().find(|w| w.id == id))
                .map(|w| w.name.clone());
            let text = match name {
                Some(name) => format!(
                    "orchestrator exited; its alerts are yours \u{b7} anthrex restart {name} restarts it"
                ),
                None => "orchestrator exited; its alerts are yours".to_owned(),
            };
            (text, since.map(|at| app.run_age(at)))
        }
    })
}

/// Milestone 9.9 decision 24: the pending `ask_user` as `(text, detail, age)`; the
/// detail is the context, then the options numbered from 1.
fn ask_text(app: &App, run: &RunInfo) -> Option<(String, String, Option<u64>)> {
    let ask = run.orchestrator.as_ref()?.ask.as_ref()?;
    let mut parts = Vec::new();
    if !ask.context.trim().is_empty() {
        parts.push(ask.context.clone());
    }
    if !ask.options.is_empty() {
        let numbered: Vec<String> = (ask.options.iter().enumerate())
            .map(|(i, option)| format!("{}. {option}", i + 1))
            .collect();
        parts.push(numbered.join("\n"));
    }
    let text = format!("orchestrator asks: {}", one_line(&ask.question));
    Some((text, parts.join("\n\n"), Some(app.run_age(ask.asked_at))))
}

/// Decision 18's priority 3 for a blocked task: while the orchestrator lives, only
/// what is user-only (milestone 9.9 decision 22); with none, every reason. A paused
/// task is never an alert, nor one at the plan gate or under a hold awaiting approval:
/// the gate's or the hold's alert covers it, and it is drawn as planned (`○`,
/// milestone 9.0.7 ruling).
fn blocked_text(run: &RunInfo, task: &proto::TaskInfo) -> Option<String> {
    if task.state != TaskState::Blocked || is_paused(task) {
        return None;
    }
    if run.state == RunState::AwaitingApproval || tree::task_held(run, task) {
        return None;
    }
    let kind = RouteKind::Blocked {
        reason: task.block.as_ref().map(|block| block.reason),
        user_only: task.block.as_ref().is_some_and(|block| block.user_only),
    };
    if alert_route(kind, orchestrator_lives(run)) == Route::Orchestrator {
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

/// Decisions 17 and 18: every alert, most urgent first — by priority, then the run's
/// `created_at` (the order `shown_runs` gives), then the order the rules list.
pub fn alerts(app: &App) -> Vec<Alert> {
    #[cfg(test)]
    built::add();
    let mut out = Vec::new();
    for run in tree::shown_runs(&app.runs.runs) {
        let id = run.run_id.clone();
        let who = AlertWho::Run {
            goal: one_line(tree::run_title(run)),
            id: one_line(&id),
        };
        // `detail` defaults to the text; `age` only where decision 8 knows it.
        let lives = orchestrator_lives(run);
        let mut push = |priority,
                        key: AlertKey,
                        text: String,
                        task: Option<&str>,
                        detail: Option<&str>,
                        age| {
            // Milestone 9.9 decision 22: what the living orchestrator handles is not listed.
            let kind = route_kind(&key, Some(run));
            if alert_route(kind, lives) == Route::Orchestrator {
                return;
            }
            let text = one_line(&text);
            out.push(Alert {
                you: is_you(kind, lives),
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
            push(1, key, text.to_owned(), None, None, age);
        }
        if let Some((text, age)) = stuck_text(app, run) {
            push(
                1,
                AlertKey::OrchestratorStuck(id.clone()),
                text,
                None,
                None,
                age,
            );
        }
        if let Some((text, detail, age)) = ask_text(app, run) {
            let key = AlertKey::OrchestratorAsks(id.clone());
            push(1, key, text, None, Some(detail.as_str()), age);
        }
        if run.doc_gate.is_some() && run.state == RunState::AwaitingApproval {
            // Milestone 9.6 (DF §2.1): a design gate's version waiting for the user; none
            // while the orchestrator revises it.
            if let Some(text) = super::doc_gate::alert_text(run) {
                push(2, AlertKey::Gate(id.clone()), text, None, None, None);
            }
        } else if run.state == RunState::AwaitingApproval {
            // Milestone 9.3 decision 32: a later round's gate counts its own tasks.
            let n = super::plan_review::review_tasks(run, &ReviewTarget::Gate).len();
            let text = match run.round {
                0 | 1 => format!("plan awaits approval · {}", tasks_text(n)),
                round => format!("round {round} awaits approval · {}", tasks_text(n)),
            };
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
        for (stage, kind, text) in stage_alerts(run) {
            let key = AlertKey::Stage {
                run: id.clone(),
                stage,
                kind,
            };
            push(3, key, text, None, None, None);
        }
        // Milestone 9.2 ruling R-13: what the user must act on for the delivery, from
        // the daemon's typed alerts (never its attention text), at priority 3.
        for (key, alert) in delivery_alerts(run) {
            push(3, key, alert.text.clone(), None, None, None);
        }
        // Milestone 9.2 ruling R-13: a `pr` run's pull requests are merged on GitHub,
        // never accepted, so a complete one is never "ready to accept".
        let pr = run
            .delivery
            .as_ref()
            .is_some_and(|d| d.mode == proto::DeliveryMode::Pr);
        if run.state == RunState::Complete && !pr {
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
            you: false,
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
    /// `C-b a` (milestone 9.0.7 decision 11): opens the Alerts view in the main pane on
    /// the first alert. A hidden sidebar stays hidden: the view does not need it.
    /// Refused while the plan review is open (9.0.5 decision 13); nothing while the
    /// view is already open (fix round 1 ruling).
    pub(super) fn focus_alerts(&mut self) -> Vec<Effect> {
        if self.alerts_focus.is_some() {
            return vec![];
        }
        if self.plan_review.is_some() {
            self.toast(super::plan_review::LEAVE_REVIEW_FIRST);
            return vec![];
        }
        let selected = alerts(self).into_iter().next().map(|alert| alert.key);
        self.alerts_focus = Some(AlertsFocus {
            selected,
            at: 0,
            scroll: 0,
        });
        self.sync_alerts_mode();
        vec![]
    }

    /// Risks 1: the keymap's alerts mode follows `alerts_focus`, here only.
    fn sync_alerts_mode(&mut self) {
        self.keymap.set_alerts_mode(self.alerts_focus.is_some());
    }

    pub(crate) fn leave_alerts(&mut self) {
        self.alerts_focus = None;
        self.sync_alerts_mode();
    }

    /// A bare key while the view is open: the selection is repaired first, then the
    /// view's keys (`app/alerts_view.rs`).
    pub(super) fn on_alerts_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        self.repair_alerts_focus();
        self.alerts_view_key(key)
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
                focus.scroll = 0;
            }
            None => {
                focus.at = focus.at.min(keys.len() - 1);
                focus.selected = Some(keys[focus.at].clone());
                // Another alert's detail starts at its top.
                focus.scroll = 0;
            }
        }
    }
}

/// Ruling R-13: a `pr` run's delivery alerts with their keys, each the `n`-th of its
/// kind and stage, so two over-the-cap threads of one stage are two alerts.
pub(crate) fn delivery_alerts(run: &RunInfo) -> Vec<(AlertKey, &proto::DeliveryAlert)> {
    let Some(d) = run
        .delivery
        .as_ref()
        .filter(|d| d.mode == proto::DeliveryMode::Pr)
    else {
        return Vec::new();
    };
    let mut out: Vec<(AlertKey, &proto::DeliveryAlert)> = Vec::new();
    for alert in &d.alerts {
        let n = (out.iter())
            .filter(|(_, a)| (a.kind, a.stage) == (alert.kind, alert.stage))
            .count();
        let key = AlertKey::Delivery {
            run: run.run_id.clone(),
            kind: alert.kind,
            stage: alert.stage,
            n,
        };
        out.push((key, alert));
    }
    out
}

/// How many times `alerts` built the list on this thread, for the final fix wave's
/// once-a-frame test (task 5's deferred minor).
#[cfg(test)]
pub(crate) mod built {
    use std::cell::Cell;
    thread_local! {
        static BUILT: Cell<usize> = const { Cell::new(0) };
    }
    pub(crate) fn add() {
        BUILT.with(|b| b.set(b.get() + 1));
    }
    /// The builds since the last `take`.
    pub(crate) fn take() -> usize {
        BUILT.with(|b| b.replace(0))
    }
}
