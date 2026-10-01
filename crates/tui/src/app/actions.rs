//! Milestone 9.0.6 §1: the action menu (decisions 10, 12–14, 16, 18). `.` on a run,
//! stage or task opens a `Modal::Action` listing the node's daemon-built actions with
//! the client's own around them; a `Confirm` action goes to its page, which sends one
//! tagged request on `y` (or Enter, unless the action is destructive-grade) and records
//! it in `App.replies`. The menu re-reads its node after every snapshot. Pure.

use super::replies::{PendingWhat, reply_timeout};
use super::{App, Effect, Modal, ToastLevel};
use crate::actions_request::{
    ActionInput, ActionTarget, moved_base_confirm, request_for, short_id,
};
use crate::theme::{Glyph, glyph};
use crate::tree::NodeKey;
use crossterm::event::{KeyCode, KeyEvent};
use proto::{
    ActionInfo, ActionKind, ActionNeeds, AgentRole, BaseMovedInfo, FinishAction, FullState,
    HoldState, RunInfo, RunRequest, RunState, TaskInfo, TaskState,
};

/// The open menu: its node, its entries and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionFlow {
    pub run_id: String,
    /// The run's goal, for its name once the run is gone.
    pub goal: String,
    pub target: ActionTarget,
    pub items: Vec<ActionInfo>,
    pub selected: usize,
    pub step: ActionStep,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionStep {
    Menu,
    Confirm(ConfirmPage),
    MovedBase(MovedBasePage),
}

/// Decision 14: the action, its effect and the details that matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmPage {
    pub info: ActionInfo,
    pub details: Vec<(String, String)>,
}

impl ConfirmPage {
    /// Decision 14 and Global Constraint 3: a destructive action, and accept, confirm
    /// on `y` only.
    pub fn y_only(&self) -> bool {
        y_only(&self.info)
    }
}

/// Decision 18: the base moved under an accept; the user types the run's short id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedBasePage {
    pub info: ActionInfo,
    pub moved: BaseMovedInfo,
    pub typed: String,
    /// Enter was pressed on a wrong id: `type <short> exactly` shows.
    pub wrong: bool,
}

/// Characters the short-id box keeps.
const TYPED_MAX: usize = 32;

fn y_only(info: &ActionInfo) -> bool {
    info.destructive || info.kind.destructive() || info.kind == ActionKind::Accept
}

fn local(kind: ActionKind, label: &str, effect: String) -> ActionInfo {
    ActionInfo {
        needs: kind.needs(),
        destructive: false,
        kind,
        label: label.to_string(),
        effect,
        refused_why: None,
    }
}

/// Decision 10: the kinds the client adds itself, in menu order relative to the
/// daemon's (`ReviewPlan` and `OpenConversation` lead, `Stats` trails).
pub fn local_actions(run: &RunInfo, target: &ActionTarget) -> Vec<ActionInfo> {
    let mut out = Vec::new();
    match target {
        ActionTarget::Run => {
            let awaiting = run.state == RunState::AwaitingApproval
                || run.holds.iter().any(|h| h.state == HoldState::Awaiting);
            if awaiting {
                let effect = "review plan: read every task before approving".to_string();
                out.push(local(ActionKind::ReviewPlan, "review plan", effect));
            }
            let effect = "stats: this project's run history".to_string();
            out.push(local(ActionKind::Stats, "stats", effect));
        }
        ActionTarget::Task(id) => {
            if task_of(run, id).is_some_and(|task| !task.rounds.is_empty()) {
                let effect = format!("open conversation: {id}'s conversation, read-only");
                out.push(local(
                    ActionKind::OpenConversation,
                    "open conversation",
                    effect,
                ));
            }
        }
        ActionTarget::Stage(_) => {}
    }
    out
}

fn task_of<'a>(run: &'a RunInfo, id: &str) -> Option<&'a TaskInfo> {
    run.tasks.iter().find(|task| task.id == id)
}

/// The node's entries, or `None` when the node is gone.
fn menu_items(run: &RunInfo, target: &ActionTarget) -> Option<Vec<ActionInfo>> {
    let daemon = match target {
        ActionTarget::Run => &run.actions,
        ActionTarget::Stage(n) => &run.stages.iter().find(|s| s.n == *n)?.actions,
        ActionTarget::Task(id) => &task_of(run, id)?.actions,
    };
    let (trailing, leading): (Vec<_>, Vec<_>) = local_actions(run, target)
        .into_iter()
        .partition(|a| a.kind == ActionKind::Stats);
    Some(
        leading
            .into_iter()
            .chain(daemon.iter().filter(|a| !a.kind.is_local()).cloned())
            .chain(trailing)
            .collect(),
    )
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn seven(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// Decision 14's details rows ("Confirm details" in the exact text table), from the run
/// alone.
pub fn confirm_details(run: &RunInfo, kind: &ActionKind, ascii: bool) -> Vec<(String, String)> {
    let row = |l: &str, v: String| (l.to_string(), v);
    let id = &run.run_id;
    match kind {
        ActionKind::Accept => {
            let mut rows: Vec<_> = run
                .stages
                .iter()
                .map(|s| {
                    let tier = match s.full.state {
                        FullState::Green => format!("{} green", glyph(Glyph::Passed, ascii)),
                        FullState::Red => format!("{} red", glyph(Glyph::Failed, ascii)),
                        FullState::None => format!("{} not run", glyph(Glyph::NotStarted, ascii)),
                        FullState::Running => "running".to_string(),
                        FullState::Bisecting => "bisecting".to_string(),
                    };
                    let sep = if ascii { "-" } else { "·" };
                    let merged = format!("{}/{} merged {sep} tier 3 {tier}", s.merged, s.tasks);
                    (format!("stage {}", s.n), merged)
                })
                .collect();
            if rows.is_empty() {
                let merged = run
                    .tasks
                    .iter()
                    .filter(|t| t.state == TaskState::Merged)
                    .count();
                rows.push(row("tasks", format!("{merged}/{} merged", run.tasks.len())));
            }
            rows.push(row(
                "base",
                format!("{}@{}", run.base_branch, seven(&run.base_sha)),
            ));
            let head = format!("{}@{}", run.run_branch, seven(&run.run_head));
            rows.push(row("merges", head));
            rows
        }
        ActionKind::Discard | ActionKind::Reject => {
            let started = run
                .tasks
                .iter()
                .filter(|t| t.start_commit.is_some())
                .count();
            let sep = if ascii { "-" } else { "·" };
            vec![
                row(
                    "removes",
                    format!(
                        "{}, the integration worktree and anthrex/{id}/* branches",
                        plural(started, "task worktree", "task worktrees")
                    ),
                ),
                row(
                    "keeps",
                    format!(
                        "{} unchanged {sep} uncommitted work under refs/anthrex/salvage/{id}/",
                        run.base_branch
                    ),
                ),
            ]
        }
        ActionKind::Cancel => {
            let unfinished: Vec<&TaskInfo> = run
                .tasks
                .iter()
                .filter(|t| !t.state.is_finished())
                .collect();
            let workers: Vec<&str> = unfinished
                .iter()
                .filter(|t| {
                    t.rounds
                        .iter()
                        .any(|r| r.role == AgentRole::Worker && r.ended_at.is_none())
                })
                .map(|t| t.id.as_str())
                .collect();
            let stops = if workers.is_empty() {
                "no worker is live".to_string()
            } else {
                let names = workers.join(", ");
                format!("{}: {names}", plural(workers.len(), "worker", "workers"))
            };
            vec![
                row("stops", stops),
                row(
                    "cancels",
                    plural(unfinished.len(), "unmerged task", "unmerged tasks"),
                ),
            ]
        }
        _ => vec![],
    }
}

impl App {
    /// Decision 12: the menu on `target` of run `target.0`, `preselect` selected when
    /// listed (else the first entry). Does nothing for a run or node that is not there.
    pub fn open_actions(
        &mut self,
        target: (String, ActionTarget),
        preselect: Option<ActionKind>,
    ) -> Vec<Effect> {
        let (run_id, target) = target;
        let Some(run) = self.runs.runs.iter().find(|r| r.run_id == run_id) else {
            return vec![];
        };
        let Some(items) = menu_items(run, &target) else {
            return vec![];
        };
        let selected = preselect
            .and_then(|kind| items.iter().position(|a| a.kind == kind))
            .unwrap_or(0);
        self.modal = Some(Modal::Action(Box::new(ActionFlow {
            goal: run.goal.clone(),
            run_id,
            target,
            items,
            selected,
            step: ActionStep::Menu,
        })));
        vec![]
    }

    /// `.` in tree mode: the menu on the selected run, stage or task node.
    pub(crate) fn open_selected_actions(&mut self) -> Vec<Effect> {
        let target = match self.tree.selected.clone() {
            Some(NodeKey::Run(run)) => (run, ActionTarget::Run),
            Some(NodeKey::Stage { run, n }) => (run, ActionTarget::Stage(n)),
            Some(NodeKey::Task { run, id }) => (run, ActionTarget::Task(id)),
            _ => return vec![],
        };
        self.open_actions(target, None)
    }

    fn page_of(&self, flow: &ActionFlow, info: ActionInfo) -> ConfirmPage {
        let details = self
            .runs
            .runs
            .iter()
            .find(|r| r.run_id == flow.run_id)
            .map(|run| confirm_details(run, &info.kind, self.settings.badges.ascii))
            .unwrap_or_default();
        ConfirmPage { info, details }
    }

    /// `Modal::Action`'s keys; the modal was taken out, and is put back unless the key
    /// closes it.
    pub(super) fn on_action_key(&mut self, mut flow: ActionFlow, key: KeyEvent) -> Vec<Effect> {
        if flow.step == ActionStep::Menu {
            return self.on_menu_key(flow, key);
        }
        let effects = match &mut flow.step {
            ActionStep::Menu => vec![],
            ActionStep::Confirm(page) => match key.code {
                KeyCode::Esc => {
                    flow.step = ActionStep::Menu;
                    vec![]
                }
                KeyCode::Enter if page.y_only() => {
                    let verb = page.info.label.clone();
                    self.toast_at(ToastLevel::Warn, format!("press y to {verb}"));
                    vec![]
                }
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    let info = page.info.clone();
                    let request = self.run_of(&flow).and_then(|run| {
                        request_for(run, &flow.target, &info.kind, &ActionInput::None)
                    });
                    match request {
                        Some(request) => match self.send_action(&flow, &info, request) {
                            Some(effects) => return effects,
                            None => vec![],
                        },
                        None => vec![],
                    }
                }
                _ => vec![],
            },
            ActionStep::MovedBase(page) => match key.code {
                KeyCode::Esc => {
                    flow.step = ActionStep::Menu;
                    vec![]
                }
                KeyCode::Backspace => {
                    page.typed.pop();
                    page.wrong = false;
                    vec![]
                }
                KeyCode::Char(c) if !c.is_control() && page.typed.chars().count() < TYPED_MAX => {
                    page.typed.push(c);
                    page.wrong = false;
                    vec![]
                }
                KeyCode::Enter if page.typed.trim() != short_id(&flow.run_id) => {
                    page.wrong = true;
                    vec![]
                }
                KeyCode::Enter => {
                    let request = RunRequest::Finish {
                        run_id: flow.run_id.clone(),
                        action: FinishAction::Accept,
                        confirm: Some(moved_base_confirm(&flow.run_id, &page.moved)),
                    };
                    let info = page.info.clone();
                    if let Some(effects) = self.send_action(&flow, &info, request) {
                        return effects;
                    }
                    vec![]
                }
                _ => vec![],
            },
        };
        self.modal = Some(Modal::Action(Box::new(flow)));
        effects
    }

    fn on_menu_key(&mut self, mut flow: ActionFlow, key: KeyEvent) -> Vec<Effect> {
        let last = flow.items.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => return vec![],
            KeyCode::Char('j') | KeyCode::Down => flow.selected = (flow.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => flow.selected = flow.selected.saturating_sub(1),
            KeyCode::Enter => {
                let info = flow.items.get(flow.selected).cloned();
                if !self.connected() {
                    self.toast_at(ToastLevel::Warn, "not connected");
                } else if let Some(info) = info {
                    if let Some(why) = &info.refused_why {
                        self.toast_at(ToastLevel::Warn, why.clone());
                    } else {
                        match info.needs {
                            ActionNeeds::Confirm => {
                                flow.step = ActionStep::Confirm(self.page_of(&flow, info));
                            }
                            ActionNeeds::Open => return self.act_locally(flow, info.kind),
                            // Task 11 builds the forms.
                            ActionNeeds::Input(_) => self.toast("not yet"),
                        }
                    }
                }
            }
            _ => {}
        }
        self.modal = Some(Modal::Action(Box::new(flow)));
        vec![]
    }

    /// Decision 10's local kinds: they change nothing in the daemon.
    fn act_locally(&mut self, flow: ActionFlow, kind: ActionKind) -> Vec<Effect> {
        match (kind, &flow.target) {
            (ActionKind::ReviewPlan, _) => self.review_from_run_view(&flow.run_id),
            (ActionKind::OpenConversation, ActionTarget::Task(id)) => {
                let key = NodeKey::Task {
                    run: flow.run_id.clone(),
                    id: id.clone(),
                };
                self.activate_run_node(key)
            }
            // Preflight F24: the stats screen is task 15's.
            _ => {
                self.toast("not yet");
                self.modal = Some(Modal::Action(Box::new(flow)));
                vec![]
            }
        }
    }

    fn run_of(&self, flow: &ActionFlow) -> Option<&RunInfo> {
        self.runs.runs.iter().find(|r| r.run_id == flow.run_id)
    }

    /// Sends `request` for `info` tagged and records it (decision 16); the menu closes.
    /// `None` (nothing sent, the flow stays) while refused or not connected.
    fn send_action(
        &mut self,
        flow: &ActionFlow,
        info: &ActionInfo,
        request: RunRequest,
    ) -> Option<Vec<Effect>> {
        if let Some(why) = &info.refused_why {
            self.toast_at(ToastLevel::Warn, why.clone());
            return None;
        }
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return None;
        }
        let timeout = reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        let what = PendingWhat::Action {
            run_id: flow.run_id.clone(),
            target: flow.target.clone(),
            kind: info.kind.clone(),
        };
        self.replies.insert(id, what, timeout);
        Some(vec![effect])
    }

    /// Decision 13, after every snapshot: the open menu re-reads its node's entries; a
    /// node that is gone closes it with a toast; a page whose action is now refused
    /// shows the new reason, and one no longer listed goes back to the menu.
    pub(super) fn follow_action_flow(&mut self) {
        let Some(Modal::Action(flow)) = self.modal.as_ref() else {
            return;
        };
        let items = self
            .run_of(flow)
            .and_then(|run| menu_items(run, &flow.target));
        let Some(items) = items else {
            let gone = match &flow.target {
                ActionTarget::Task(id) if self.run_of(flow).is_some() => id.clone(),
                ActionTarget::Stage(n) if self.run_of(flow).is_some() => format!("stage {n}"),
                _ => {
                    let p = self.palette();
                    crate::ui::kit::run_name_in(&flow.goal, &flow.run_id, crate::ui::kit::WRAP, p)
                }
            };
            self.modal = None;
            self.toast(format!("{gone} is gone"));
            return;
        };
        let Some(Modal::Action(mut flow)) = self.modal.take() else {
            return;
        };
        let kept = flow.items.get(flow.selected).map(|a| a.kind.clone());
        flow.selected = kept
            .and_then(|kind| items.iter().position(|a| a.kind == kind))
            .unwrap_or(flow.selected)
            .min(items.len().saturating_sub(1));
        flow.items = items;
        let current = match &flow.step {
            ActionStep::Menu => None,
            ActionStep::Confirm(page) => Some(page.info.clone()),
            ActionStep::MovedBase(page) => Some(page.info.clone()),
        };
        if let Some(old) = current {
            match flow.items.iter().find(|a| a.kind == old.kind).cloned() {
                Some(info) => {
                    if let ActionStep::MovedBase(page) = &mut flow.step {
                        page.info = info;
                    } else {
                        let page = self.page_of(&flow, info);
                        flow.step = ActionStep::Confirm(page);
                    }
                }
                None => {
                    flow.step = ActionStep::Menu;
                    let text = format!("{} is no longer available", old.label);
                    self.toast_at(ToastLevel::Warn, text);
                }
            }
        }
        self.modal = Some(Modal::Action(flow));
    }
}
