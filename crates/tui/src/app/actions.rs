//! Milestone 9.0.6 §1: the action menu (decisions 10, 12–14, 16, 18). `.` on a run,
//! stage or task opens a `Modal::Action` listing the node's daemon-built actions with
//! the client's own around them; a `Confirm` action goes to its page, which sends one
//! tagged request on `y` (or Enter, unless the action is destructive-grade) and records
//! it in `App.replies`. The menu re-reads its node after every snapshot. Pure.

#[path = "action_forms.rs"]
pub(crate) mod forms;

use super::replies::{PendingWhat, reply_timeout};
use super::{App, Effect, Modal, ToastLevel};
use crate::actions_request::{
    ActionInput, ActionTarget, moved_base_confirm, request_for, short_id,
};
use crate::theme::{Glyph, glyph};
use crate::tree::NodeKey;
use crossterm::event::{KeyCode, KeyEvent};
use forms::ActionForm;
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
    /// Decision 15: an input form; Enter goes to its page, Esc back to the menu.
    Form(Box<ActionForm>),
    Confirm(ConfirmPage),
    MovedBase(MovedBasePage),
}

/// Decision 14: the action, its effect and the details that matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmPage {
    pub info: ActionInfo,
    pub details: Vec<(String, String)>,
    /// The form this page confirms: what it sends, and where Esc goes back to.
    pub form: Option<Box<ActionForm>>,
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
            // Milestone 9.6: a brainstorm or spec gate is reviewed with `review document`.
            let awaiting = (run.state == RunState::AwaitingApproval
                && super::doc_gate::doc_gate_of(run).is_none())
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

/// Milestone 9.3: round 2 or later, not yet ended, as the daemon's
/// `goal_rounds_end::open_round` reads it: by `RoundInfo.ended` (its `ended_at`), not
/// the outcome, which a cancelled round has while it still runs (final fix wave C-m3).
pub(crate) fn open_round(run: &RunInfo) -> bool {
    run.round > 1 && run.rounds.last().is_some_and(|r| !r.ended)
}

/// Decision 12: the tasks a reject of the open round cancels, its unfinished ones.
/// The action menu's reject page and the plan gate's `x` both count them.
pub(crate) fn round_open_tasks(run: &RunInfo) -> usize {
    (run.tasks.iter())
        .filter(|t| t.round == run.round && !t.state.is_finished())
        .count()
}

fn task_of<'a>(run: &'a RunInfo, id: &str) -> Option<&'a TaskInfo> {
    run.tasks.iter().find(|task| task.id == id)
}

/// The node's entries, or `None` when the node is gone.
pub(crate) fn menu_items(run: &RunInfo, target: &ActionTarget) -> Option<Vec<ActionInfo>> {
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
        // Milestone 9.3 decision 12: an open later round's reject drops that round.
        ActionKind::Reject if open_round(run) => {
            let sep = if ascii { "-" } else { "·" };
            let n = run.round;
            let tasks = plural(round_open_tasks(run), "task", "tasks");
            vec![
                row("cancels", format!("round {n}'s {tasks}")),
                row(
                    "keeps",
                    format!("the earlier rounds {sep} {} unchanged", run.base_branch),
                ),
            ]
        }
        ActionKind::Discard | ActionKind::Reject => {
            // No count: the snapshot cannot tell which worktrees still exist (merged
            // and cancelled tasks' are already gone), and a destructive page never
            // states a false number (controller ruling, fix round 1).
            let sep = if ascii { "-" } else { "·" };
            vec![
                row(
                    "removes",
                    format!("the run's remaining worktrees and its anthrex/{id}/* branches"),
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
            vec![
                row("stops", cancel_stops(run)),
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

    pub(super) fn page_of(&self, flow: &ActionFlow, info: ActionInfo) -> ConfirmPage {
        let details = self
            .runs
            .runs
            .iter()
            .find(|r| r.run_id == flow.run_id)
            .map(|run| confirm_details(run, &info.kind, self.settings.badges.ascii))
            .unwrap_or_default();
        ConfirmPage {
            info,
            details,
            form: None,
        }
    }

    /// `Modal::Action`'s keys; the modal was taken out, and is put back unless the key
    /// closes it.
    pub(super) fn on_action_key(&mut self, mut flow: ActionFlow, key: KeyEvent) -> Vec<Effect> {
        if flow.step == ActionStep::Menu {
            return self.on_menu_key(flow, key);
        }
        match std::mem::replace(&mut flow.step, ActionStep::Menu) {
            ActionStep::Form(form) => {
                self.on_form_key(&mut flow, form, key);
                self.modal = Some(Modal::Action(Box::new(flow)));
                return vec![];
            }
            other => flow.step = other,
        }
        let effects = match &mut flow.step {
            ActionStep::Menu | ActionStep::Form(_) => vec![],
            ActionStep::Confirm(page) => match key.code {
                KeyCode::Esc => {
                    let form = page.form.take();
                    flow.step = form.map_or(ActionStep::Menu, ActionStep::Form);
                    vec![]
                }
                KeyCode::Enter if page.y_only() => {
                    let verb = page.info.label.clone();
                    self.toast_at(ToastLevel::Warn, format!("press y to {verb}"));
                    vec![]
                }
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    let info = page.info.clone();
                    let input = match &page.form {
                        Some(form) => form.input(),
                        None => Ok(ActionInput::None),
                    };
                    let input = match input {
                        Ok(input) => input,
                        Err(why) => {
                            self.toast_at(ToastLevel::Warn, why);
                            self.modal = Some(Modal::Action(Box::new(flow)));
                            return vec![];
                        }
                    };
                    let request = self
                        .run_of(&flow)
                        .and_then(|run| request_for(run, &flow.target, &info.kind, &input));
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
                            ActionNeeds::Input(kind) => {
                                let effects = self.open_form(&mut flow, info, kind);
                                self.modal = Some(Modal::Action(Box::new(flow)));
                                return effects;
                            }
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
            (ActionKind::ReviewDoc, _) => self.open_doc_gate(&flow.run_id),
            (ActionKind::OpenConversation, ActionTarget::Task(id)) => {
                let key = NodeKey::Task {
                    run: flow.run_id.clone(),
                    id: id.clone(),
                };
                self.activate_run_node(key)
            }
            // Decision 38: the stats screen on the run's project; the menu closes.
            (ActionKind::Stats, _) => match self.run_of(&flow).map(|r| r.project.clone()) {
                Some(dir) => self.open_stats(dir),
                None => vec![],
            },
            // Milestone 9.3 decision 32: the iterate dialog replaces the menu.
            (ActionKind::Iterate, ActionTarget::Run) => self.open_iterate(&flow.run_id),
            // A kind without a local screen: a `not yet` toast; the menu stays open.
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
            ActionStep::Form(form) => Some(form.info().clone()),
            ActionStep::Confirm(page) => Some(page.info.clone()),
            ActionStep::MovedBase(page) => Some(page.info.clone()),
        };
        if let Some(old) = current {
            match flow.items.iter().find(|a| a.kind == old.kind).cloned() {
                Some(info) => match &mut flow.step {
                    ActionStep::MovedBase(page) => page.info = info,
                    ActionStep::Form(form) => form.set_info(info),
                    _ => {
                        let form = match &mut flow.step {
                            ActionStep::Confirm(page) => page.form.take(),
                            _ => None,
                        };
                        let mut page = self.page_of(&flow, info);
                        if let Some(mut form) = form {
                            form.set_info(page.info.clone());
                            page.details
                                .extend(form.details(self.settings.badges.ascii));
                            page.form = Some(form);
                        }
                        flow.step = ActionStep::Confirm(page);
                    }
                },
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

/// The Cancel confirm's `stops` row: each live session that writes an unfinished task
/// (whole-branch review D, I-1: a racer, `t1 racer a`, and a test writer, `t1 test
/// writer`, as well as a worker, `t1`).
pub(crate) fn cancel_stops(run: &RunInfo) -> String {
    let workers: Vec<String> = (run.tasks.iter())
        .filter(|t| !t.state.is_finished())
        .flat_map(|t| {
            let mut live: Vec<_> = (t.rounds.iter())
                .filter(|r| crate::inspector::writes(r.role) && r.ended_at.is_none())
                .collect();
            live.sort_by_key(|r| r.lane);
            live.into_iter().map(|r| match (r.role, r.lane) {
                (AgentRole::Racer, Some(lane)) => format!("{} racer {}", t.id, lane.label()),
                (AgentRole::TestWriter, _) => format!("{} test writer", t.id),
                _ => t.id.clone(),
            })
        })
        .collect();
    if workers.is_empty() {
        return "no worker is live".to_string();
    }
    let names = workers.join(", ");
    format!("{}: {names}", plural(workers.len(), "worker", "workers"))
}
