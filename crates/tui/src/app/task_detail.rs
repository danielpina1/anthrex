//! Milestone 9.0.5 decisions 23 and 25: the task panel's on-demand detail (the brief,
//! the acceptance and the worker's summary, which the snapshot does not carry), and
//! the panel's scroll and brief expansion. One tagged `TaskDetail` goes out when the
//! inspected task, or its key, changes — once when it is selected, once per turn,
//! round, review or merge, never on a timer. Pure: requests are `Effect`s.

use super::{App, Effect};
use crate::tree::NodeKey;
use proto::{AgentRole, RunRequest, TaskDetailInfo, TaskInfo, TaskState};

/// Decision 23's key: what, when it changes, makes a fetched detail stale.
pub type DetailKey = (TaskState, usize, usize, u32, bool, bool, bool);

/// Decision 23: `TaskInfo`'s key.
pub fn detail_key(task: &TaskInfo) -> DetailKey {
    let workers = task
        .rounds
        .iter()
        .filter(|round| round.role == AgentRole::Worker);
    let turns = workers.clone().map(|round| round.turns).sum();
    let latest_open = workers
        .max_by_key(|round| (round.started_at, round.session, round.round))
        .is_some_and(|round| round.turn_open);
    (
        task.state,
        task.rounds.len(),
        task.reviews.len(),
        turns,
        latest_open,
        task.done_signal.is_some(),
        task.merge_commit.is_some(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailState {
    InFlight(u64),
    Ready(Box<TaskDetailInfo>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDetailCache {
    pub run_id: String,
    pub task_id: String,
    pub key: DetailKey,
    pub state: DetailState,
}

impl App {
    /// The run view's selected task while the panel is showing, with its key.
    fn inspected_task(&self) -> Option<(String, String, DetailKey)> {
        let view = self.run_view.as_ref()?;
        if !self.overview || !self.inspector_visible {
            return None;
        }
        let Some(NodeKey::Task { run, id }) = &self.tree.selected else {
            return None;
        };
        if *run != view.run_id {
            return None;
        }
        let info = self.runs.runs.iter().find(|r| r.run_id == *run)?;
        let task = info.tasks.iter().find(|task| task.id == *id)?;
        Some((run.clone(), id.clone(), detail_key(task)))
    }

    /// `on_tick`'s share of decision 23: at most one request, and only when the cache
    /// names another task or another key.
    pub(super) fn check_task_detail(&mut self) -> Option<Effect> {
        if !self.connected() {
            return None;
        }
        let (run_id, task_id, key) = self.inspected_task()?;
        let current = self.task_detail.as_ref().is_some_and(|cache| {
            cache.run_id == run_id && cache.task_id == task_id && cache.key == key
        });
        if current {
            return None;
        }
        let request = RunRequest::TaskDetail {
            run_id: run_id.clone(),
            task_id: task_id.clone(),
        };
        let (id, effect) = self.tagged_request(request);
        self.task_detail = Some(TaskDetailCache {
            run_id,
            task_id,
            key,
            state: DetailState::InFlight(id),
        });
        Some(effect)
    }

    fn detail_waiting_on(&mut self, request_id: Option<u64>) -> Option<&mut TaskDetailCache> {
        let cache = self.task_detail.as_mut()?;
        match cache.state {
            DetailState::InFlight(id) if Some(id) == request_id => Some(cache),
            _ => None,
        }
    }

    /// The reply carrying the in-flight request's id fills the cache; any other is
    /// dropped.
    pub(super) fn on_task_detail(&mut self, detail: Box<TaskDetailInfo>, request_id: Option<u64>) {
        if let Some(cache) = self.detail_waiting_on(request_id) {
            cache.state = DetailState::Ready(detail);
        }
    }

    /// A refusal of the in-flight request: shown in the panel, not toasted. `true`
    /// when it was that request's.
    pub(super) fn on_task_detail_refused(
        &mut self,
        message: &str,
        request_id: Option<u64>,
    ) -> bool {
        let Some(cache) = self.detail_waiting_on(request_id) else {
            return false;
        };
        let line = message
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("refused");
        cache.state = DetailState::Failed(crate::safe_text::one_line(line));
        true
    }

    /// The connection refused the in-flight request's send.
    pub(super) fn task_detail_not_sent(&mut self, id: u64) {
        if let Some(cache) = self.detail_waiting_on(Some(id)) {
            cache.state = DetailState::Failed("the detail request was not sent".into());
        }
    }

    /// A lost link took the reply with it: an in-flight request is forgotten, so the
    /// next tick after reconnecting asks again.
    pub(super) fn forget_task_detail_in_flight(&mut self) {
        if matches!(
            self.task_detail.as_ref().map(|cache| &cache.state),
            Some(DetailState::InFlight(_))
        ) {
            self.task_detail = None;
        }
    }

    /// The detail the panel shows for `run_id`'s `task_id`: its cache, when it is that
    /// task's. A newer key's request replaces the cache, so the brief reads `loading…`
    /// for that request's round trip (decision 23 keeps one state, not two).
    pub(crate) fn task_detail_for(&self, run_id: &str, task_id: &str) -> Option<&DetailState> {
        self.task_detail
            .as_ref()
            .filter(|cache| cache.run_id == run_id && cache.task_id == task_id)
            .map(|cache| &cache.state)
    }

    /// Decision 25: the scroll, honoured only while its node is the selection.
    pub(crate) fn inspector_scroll_for(&self, key: &NodeKey) -> u16 {
        match &self.inspector_scroll {
            Some((node, scroll)) if node == key => *scroll,
            _ => 0,
        }
    }

    /// Decision 25: the brief is expanded only while its node is the selection.
    pub(crate) fn brief_expanded_for(&self, key: &NodeKey) -> bool {
        self.brief_expanded.as_ref() == Some(key)
    }

    /// Decision 25: "a new selection starts at the top, collapsed". Drops the scroll
    /// and the expansion once their node is no longer the selection, so leaving a task
    /// and coming back does not restore them. Called at the start of every key, click
    /// and tick: any return to the old node takes one of those first.
    pub(crate) fn forget_stale_panel_state(&mut self) {
        let selected = self.tree.selected.as_ref();
        if self
            .inspector_scroll
            .as_ref()
            .is_some_and(|(node, _)| Some(node) != selected)
        {
            self.inspector_scroll = None;
        }
        if self
            .brief_expanded
            .as_ref()
            .is_some_and(|node| Some(node) != selected)
        {
            self.brief_expanded = None;
        }
    }

    /// Decision 25's keys in the run view with a task selected: `PageUp`/`PageDown`
    /// scroll the panel by its interior minus one, clamped by the renderer's own row
    /// count; `b` expands or collapses the brief. `false`: no task is selected.
    pub(super) fn on_task_panel_key(&mut self, code: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode;
        let Some(key @ NodeKey::Task { .. }) = self.tree.selected.clone() else {
            return false;
        };
        match code {
            KeyCode::Char('b') => {
                self.brief_expanded = if self.brief_expanded_for(&key) {
                    None
                } else {
                    Some(key)
                };
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                let (width, height) = self.task_panel_interior();
                let body = height.saturating_sub(1);
                let rows = crate::inspector::task_panel_rows(self, width);
                let max = u16::try_from(rows).unwrap_or(u16::MAX).saturating_sub(body);
                let page = height.saturating_sub(1).max(1);
                let from = self.inspector_scroll_for(&key).min(max);
                let scroll = if code == KeyCode::PageDown {
                    from.saturating_add(page).min(max)
                } else {
                    from.saturating_sub(page)
                };
                self.inspector_scroll = Some((key, scroll));
            }
            _ => return false,
        }
        true
    }

    /// The run view's panel interior (width, height), from the last frame's overview
    /// area: the footer `ui::overview::areas` gives, less its border and padding.
    pub(super) fn task_panel_interior(&self) -> (u16, u16) {
        let Some(main) = self.graph_main else {
            return (0, 0);
        };
        let (_, footer) =
            crate::ui::overview::areas(main, self.inspector_visible, self.run_view.is_some());
        (
            footer.width.saturating_sub(4),
            footer.height.saturating_sub(2),
        )
    }
}
