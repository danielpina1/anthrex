//! Task M8c.6: Enter on a run node (decisions 22, 24 and 25). The root focuses the
//! orchestrator's PTY window, the one place the user types; every other agent opens its
//! conversation, read-only, and nothing here ever focuses a headless window.

use super::{App, Effect};
use crate::tree::{self, NodeKey, RowKind};
use proto::{PlannerState, ScoutState, WindowKind};

/// Where an agent's conversation would come from: its window, a label for the toasts,
/// and whether it has ended.
struct Agent {
    window: Option<u32>,
    label: String,
    ended: bool,
}

impl App {
    /// Enter, or a double click, on a run-view node, and on a project tree's `Run` node.
    pub(crate) fn activate_run_node(&mut self, key: NodeKey) -> Vec<Effect> {
        let agent = match key {
            NodeKey::Run(id) => {
                let inside = self.run_view.as_ref().is_some_and(|view| view.run_id == id);
                if !inside {
                    self.open_run_view(id);
                    return vec![];
                }
                return self.enter_run_root(&id);
            }
            NodeKey::Task { run, id } => match self.task_agent(&run, &id) {
                Some(agent) => agent,
                None => {
                    self.toast(format!("{id} has no agent yet"));
                    return vec![];
                }
            },
            // Milestone 9.1 decision 55: a stage has no agent; Enter folds it.
            key @ NodeKey::Stage { .. } => {
                self.toggle_tree_node(&key);
                return vec![];
            }
            NodeKey::Planner { run, epic } => {
                let planner = self
                    .run_info(&run)
                    .and_then(|info| info.planners.iter().find(|p| p.epic == epic));
                let Some(planner) = planner else {
                    return vec![];
                };
                Agent {
                    window: planner.window_id,
                    label: format!("planner {epic}"),
                    ended: planner.ended_at.is_some() || planner.state != PlannerState::Planning,
                }
            }
            NodeKey::Scout { run, id } => {
                let scout = self
                    .run_info(&run)
                    .and_then(|info| info.scouts.iter().find(|s| s.id == id));
                let Some(scout) = scout else {
                    return vec![];
                };
                Agent {
                    window: scout.window_id,
                    label: format!("scout {id}"),
                    ended: scout.ended_at.is_some()
                        || matches!(scout.state, ScoutState::Reported | ScoutState::Failed),
                }
            }
            NodeKey::AgentRound {
                run,
                task,
                role,
                session,
                round,
            } => {
                let info = self
                    .run_info(&run)
                    .and_then(|info| info.tasks.iter().find(|t| t.id == task));
                let Some(info) = info else {
                    return vec![];
                };
                let rounds = tree::display_rounds(info, &self.windows);
                let found = rounds.iter().find(|shown| {
                    shown.info.role == role
                        && shown.info.session == session
                        && shown.number == round
                });
                let Some(shown) = found else {
                    return vec![];
                };
                Agent {
                    window: shown.info.window_id,
                    label: tree::round_label(role, session, round),
                    ended: shown.info.ended_at.is_some(),
                }
            }
            // Windows, sub-agents and projects are `activate_tree_node`'s own arms.
            NodeKey::Project(_) | NodeKey::Window(_) | NodeKey::Subagent { .. } => {
                return vec![];
            }
        };
        self.open_agent(agent)
    }

    /// Decision 24, the root: focus the orchestrator's window and leave tree mode, as
    /// Enter on a window does; no orchestrator window (every run until M9, and the fast
    /// path) toasts why.
    fn enter_run_root(&mut self, id: &str) -> Vec<Effect> {
        let rows = self.nav_rows();
        let orchestrator = rows.first().and_then(|row| match &row.kind {
            RowKind::Run { orchestrator, .. } => orchestrator.map(|w| (w.id, w.kind)),
            _ => None,
        });
        match orchestrator {
            None => {
                self.toast(format!(
                    "run {id} has no orchestrator window; Enter on an agent opens its conversation"
                ));
                vec![]
            }
            // Decision 26: never focus a headless window, whatever its role says.
            Some((window, WindowKind::Headless)) => self.open_conversation(window),
            Some((window, WindowKind::Pty)) => {
                let effects = self.focus(window);
                self.exit_tree();
                effects
            }
        }
    }

    /// Decision 24, a task: its current round — the live one with the latest start, else
    /// the latest round whose window is still listed. `None`: there is none, or that
    /// round has no window yet.
    fn task_agent(&self, run: &str, id: &str) -> Option<Agent> {
        let task = self.run_info(run)?.tasks.iter().find(|t| t.id == id)?;
        let listed = |window: Option<u32>| {
            window.is_some_and(|window| self.windows.iter().any(|w| w.id == window))
        };
        let current = task
            .rounds
            .iter()
            .filter(|round| round.ended_at.is_none())
            .max_by_key(|round| round.started_at)
            .or_else(|| {
                task.rounds
                    .iter()
                    .filter(|round| listed(round.window_id))
                    .max_by_key(|round| round.started_at)
            })?;
        Some(Agent {
            window: Some(current.window_id?),
            label: id.to_owned(),
            ended: current.ended_at.is_some(),
        })
    }

    fn run_info(&self, run: &str) -> Option<&proto::RunInfo> {
        self.runs.runs.iter().find(|info| info.run_id == run)
    }

    /// Decision 25: the agent's conversation, or the toast that says why there is none.
    fn open_agent(&mut self, agent: Agent) -> Vec<Effect> {
        let Agent {
            window,
            label,
            ended,
        } = agent;
        match window {
            None => self.toast(format!("{label} has no window yet")),
            Some(id) if self.windows.iter().any(|w| w.id == id) => {
                return self.open_conversation(id);
            }
            Some(_) if ended => self.toast(format!("{label} has finished and its window is gone")),
            Some(id) => self.toast(format!("window #{id} is not listed yet")),
        }
        vec![]
    }
}
