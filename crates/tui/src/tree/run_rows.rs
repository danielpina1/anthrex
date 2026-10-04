//! The run view's rows (milestone 8c decisions 12–15 and 21): a run's scouts, planners,
//! tasks and agent rounds as one tree, filtered and folded, below the run as its root.
//!
//! The tree is built whole first, in row order, then filtered, then walked into rows, so
//! the guides of a row always come from its *visible* siblings, as in the project tree.

use super::rows::{SubagentWalk, emit_subagents, guide_prefix};
use super::runs::orchestrator_of;
use super::task_rounds::rounds_with;
use super::{
    NodeKey, Row, RowKind, SubagentNode, TreeState, matches_filter, subagent_branch_matches,
    subagent_forest,
};
use proto::{
    AgentRole, PlannerInfo, RaceLane, RunInfo, Runtime, ScoutInfo, ScoutState, TaskInfo, TaskState,
    WindowInfo,
};
use std::collections::{HashMap, HashSet};

/// The run view's `f` filter (decision 21).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunFilter {
    #[default]
    All,
    Running,
    Blocked,
    Runtime(Runtime),
}

/// An agent round's label: `worker #1`, `worker #1 r2`, `review #2` (decision 14).
/// `number` is the display round; the orchestrator role, unreachable on a task, is
/// labelled by its session so nothing is dropped. [`round_label_with_lane`] with no lane.
pub fn round_label(role: AgentRole, session: u32, number: u32) -> String {
    round_label_with_lane(role, None, session, number)
}

/// [`round_label`] with a racing task's lane (milestone 9.5 decision 29, Interfaces
/// "Run view"): `racer a`, `racer a r2`, `review b#1`. Every other role ignores it.
pub fn round_label_with_lane(
    role: AgentRole,
    lane: Option<RaceLane>,
    session: u32,
    number: u32,
) -> String {
    // ` a`, or nothing without a lane.
    let lane = lane.map_or(String::new(), |lane| format!(" {}", lane.label()));
    match role {
        AgentRole::Worker if number > 1 => format!("worker #{session} r{number}"),
        AgentRole::Worker => format!("worker #{session}"),
        AgentRole::Reviewer => format!("review {}#{number}", lane.trim_start()),
        AgentRole::Orchestrator => format!("orchestrator #{session}"),
        // Milestone 9 decision 35: a scout round on a task is a research session.
        AgentRole::Scout => format!("research #{session}"),
        AgentRole::Planner => format!("planner #{session}"),
        AgentRole::Decider => "decider".to_string(),
        AgentRole::Racer if number > 1 => format!("racer{lane} r{number}"),
        AgentRole::Racer => format!("racer{lane}"),
        AgentRole::TestWriter if number > 1 => format!("test writer #{session} r{number}"),
        AgentRole::TestWriter => format!("test writer #{session}"),
        // Milestone 9.6: design agents have no task rounds; named for a stray one.
        AgentRole::Brainstormer => format!("brainstormer #{session}"),
        AgentRole::DocReviewer => format!("doc reviewer #{session}"),
    }
}

/// The run view of `run`: its root, then its tiers (decision 12) in sibling order
/// (decision 13), filtered by the text filter `state.filter` and by `filter`
/// (decision 21). Folds are honoured only while no filter is on, as in the project tree.
pub fn run_rows<'a>(
    run: &'a RunInfo,
    windows: &'a [WindowInfo],
    state: &TreeState,
    filter: RunFilter,
) -> Vec<Row<'a>> {
    let text = state.filter.to_lowercase();
    let filters = Filters {
        text: &text,
        run: filter,
    };
    let mut tree = build_tree(run, windows, state.keep_finished_secs);
    let inherit = Inherit::of(&tree, &filters);
    tree.children = filter_children(std::mem::take(&mut tree.children), inherit, &filters);
    let mut rows = Vec::new();
    let walk = Walk {
        state,
        filters: &filters,
    };
    emit(&mut rows, tree, &walk, &mut Vec::new(), None);
    rows
}

/// One node of the run view before it becomes a row.
pub(super) struct Node<'a> {
    pub(super) row: Row<'a>,
    pub(super) children: Vec<Node<'a>>,
    /// The window whose sub-agents hang below this node, after its children.
    subagents: Option<Subagents<'a>>,
}

struct Subagents<'a> {
    window: &'a WindowInfo,
    forest: Vec<SubagentNode<'a>>,
    /// Every sub-agent is shown, not only the branches the text filter matches.
    show_all: bool,
}

pub(super) fn node(key: NodeKey, kind: RowKind<'_>) -> Node<'_> {
    Node {
        row: Row {
            key,
            guides: String::new(),
            depth: 0,
            kind,
        },
        children: Vec::new(),
        subagents: None,
    }
}

/// Builds nodes in row order and hands each listed window's sub-agents to the first
/// node, in that order, that names it — the orchestrator's never (decision 12) — so a
/// window's sub-agents appear at most once.
struct Builder<'a> {
    run: &'a RunInfo,
    /// The listed windows by id, the first of a repeated id kept, as a scan would find.
    windows: HashMap<u32, &'a WindowInfo>,
    keep_finished_secs: u64,
    claimed: HashSet<u32>,
}

impl<'a> Builder<'a> {
    fn subagents(&mut self, window: Option<&'a WindowInfo>) -> Option<Subagents<'a>> {
        let window = window?;
        self.claimed.insert(window.id).then(|| Subagents {
            window,
            forest: subagent_forest(&window.subagents, self.keep_finished_secs),
            show_all: true,
        })
    }

    fn window(&self, id: Option<u32>) -> Option<&'a WindowInfo> {
        id.and_then(|id| self.windows.get(&id).copied())
    }

    fn scout(&mut self, scout: &'a ScoutInfo) -> Node<'a> {
        let window = self.window(scout.window_id);
        let key = NodeKey::Scout {
            run: self.run.run_id.clone(),
            id: scout.id.clone(),
        };
        let run = self.run;
        let mut scout_node = node(key, RowKind::Scout { run, scout, window });
        scout_node.subagents = self.subagents(window);
        scout_node
    }

    fn task(&mut self, task: &'a TaskInfo) -> Node<'a> {
        let run = self.run;
        let key = NodeKey::Task {
            run: run.run_id.clone(),
            id: task.id.clone(),
        };
        let mut task_node = node(key, RowKind::Task { run, task });
        let windows = &self.windows;
        for round in rounds_with(task, |id| windows.get(&id).copied()) {
            let key = NodeKey::AgentRound {
                run: run.run_id.clone(),
                task: task.id.clone(),
                role: round.info.role,
                lane: round.info.lane,
                session: round.info.session,
                round: round.number,
            };
            let window = round.window.filter(|_| round.last);
            let mut round_node = node(key, RowKind::AgentRound { run, task, round });
            round_node.subagents = self.subagents(window);
            task_node.children.push(round_node);
        }
        task_node
    }

    fn planner(&mut self, planner: &'a PlannerInfo, tasks: &[Planned<'a>]) -> Node<'a> {
        let run = self.run;
        let key = NodeKey::Planner {
            run: run.run_id.clone(),
            epic: planner.epic.clone(),
        };
        let mut planner_node = node(key, RowKind::Planner { run, planner });
        planner_node.children = tasks
            .iter()
            .map(|planned| self.task(planned.task))
            .collect();
        let window = self.window(planner.window_id);
        planner_node.subagents = self.subagents(window);
        planner_node
    }
}

/// A task with its plan index, the second half of its sibling order (decision 13).
#[derive(Clone, Copy)]
struct Planned<'a> {
    index: usize,
    task: &'a TaskInfo,
}

impl Planned<'_> {
    fn order(&self) -> (u32, usize) {
        (self.task.wave, self.index)
    }
}

/// A child of the root after the scouts: a task no planner owns, or a planner with
/// its tasks.
enum Entry<'a> {
    Task(Planned<'a>),
    Planner {
        position: usize,
        planner: &'a PlannerInfo,
        tasks: Vec<Planned<'a>>,
    },
}

impl Entry<'_> {
    /// The stage an entry hangs under: a planner's first task's, else 1.
    fn stage(&self) -> u16 {
        match self {
            Entry::Task(planned) => planned.task.stage,
            Entry::Planner { tasks, .. } => tasks.first().map_or(1, |first| first.task.stage),
        }
    }

    /// A planner sorts as its earliest task; one with no task after every root task,
    /// in `RunInfo.planners` order.
    fn order(&self) -> (bool, (u32, usize), usize) {
        match self {
            Entry::Task(planned) => (false, planned.order(), 0),
            Entry::Planner {
                position, tasks, ..
            } => match tasks.first() {
                Some(first) => (false, first.order(), 0),
                None => (true, (0, 0), *position),
            },
        }
    }
}

fn build_tree<'a>(run: &'a RunInfo, windows: &'a [WindowInfo], keep: u64) -> Node<'a> {
    let orchestrator = orchestrator_of(run, windows);
    let mut by_id = HashMap::new();
    for window in windows {
        by_id.entry(window.id).or_insert(window);
    }
    let mut builder = Builder {
        run,
        windows: by_id,
        keep_finished_secs: keep,
        claimed: orchestrator.iter().map(|window| window.id).collect(),
    };

    // A repeated id, epic or scout keeps its first copy, so no key repeats.
    let mut epics = HashSet::new();
    let planners: Vec<&PlannerInfo> = (run.planners.iter())
        .filter(|planner| epics.insert(planner.epic.as_str()))
        .collect();
    let mut ids = HashSet::new();
    let mut owned: HashMap<&str, Vec<Planned<'a>>> = HashMap::new();
    let mut entries = Vec::new();
    for (index, task) in run.tasks.iter().enumerate() {
        if !ids.insert(task.id.as_str()) {
            continue;
        }
        let planned = Planned { index, task };
        match task.epic.as_deref().filter(|epic| epics.contains(epic)) {
            Some(epic) => owned.entry(epic).or_default().push(planned),
            None => entries.push(Entry::Task(planned)),
        }
    }
    for (position, planner) in planners.into_iter().enumerate() {
        let mut tasks = owned.remove(planner.epic.as_str()).unwrap_or_default();
        tasks.sort_by_key(Planned::order);
        entries.push(Entry::Planner {
            position,
            planner,
            tasks,
        });
    }
    entries.sort_by_key(Entry::order);

    let mut scouts: Vec<&ScoutInfo> = run.scouts.iter().collect();
    scouts.sort_by(|left, right| (left.started_at, &left.id).cmp(&(right.started_at, &right.id)));
    let mut scout_ids = HashSet::new();
    scouts.retain(|scout| scout_ids.insert(scout.id.as_str()));

    let root_kind = RowKind::Run {
        run,
        orchestrator,
        position: None,
    };
    let mut root = node(NodeKey::Run(run.run_id.clone()), root_kind);
    root.children = scouts
        .into_iter()
        .map(|scout| builder.scout(scout))
        .collect();
    // Milestone 9.1 decision 55: a staged run hangs each entry under its stage node,
    // a planner under its first task's; a stage the snapshot does not list, never.
    let mut stages: Vec<Node<'a>> = Vec::new();
    if run.stages.len() > 1 {
        let mut seen = HashSet::new();
        for stage in run.stages.iter().filter(|stage| seen.insert(stage.n)) {
            let key = NodeKey::Stage {
                run: run.run_id.clone(),
                n: stage.n,
            };
            stages.push(node(key, RowKind::Stage { run, stage }));
        }
    }
    for entry in entries {
        let stage = entry.stage();
        let built = match entry {
            Entry::Task(planned) => builder.task(planned.task),
            Entry::Planner { planner, tasks, .. } => builder.planner(planner, &tasks),
        };
        let parent = stages.iter_mut().find(
            |node| matches!(node.row.kind, RowKind::Stage { stage: info, .. } if info.n == stage),
        );
        match parent {
            Some(parent) => parent.children.push(built),
            None => root.children.push(built),
        }
    }
    // Milestone 9.3 decision 32: with several rounds, each under its separator.
    root.children = super::round_rows::group(run, std::mem::take(&mut root.children), stages);
    root
}

struct Filters<'f> {
    /// The text filter, lowercased; empty when off.
    text: &'f str,
    run: RunFilter,
}

impl Filters<'_> {
    fn active(&self) -> bool {
        !self.text.is_empty() || self.run != RunFilter::All
    }
}

/// Whether an ancestor matched a filter by itself, which keeps everything below it for
/// that filter: every descendant of a text match, every round of a task the `f` filter
/// kept (decision 21).
#[derive(Clone, Copy, Default)]
struct Inherit {
    text: bool,
    run: bool,
}

impl Inherit {
    /// What the root passes down: the root is kept whatever it matches, and only its
    /// text can match, since `f` never selects the run itself.
    fn of(root: &Node<'_>, filters: &Filters<'_>) -> Self {
        Self {
            text: text_matches(root, filters.text),
            run: false,
        }
    }
}

fn text_matches(node: &Node<'_>, text: &str) -> bool {
    !text.is_empty() && matches_filter(&crate::graph::content_text(&node.row), text)
}

/// Decision 21's `f` filter on a node's own fields. The root and planners never match
/// by themselves; they are kept as ancestors.
fn run_filter_matches(kind: &RowKind<'_>, filter: RunFilter) -> bool {
    match (filter, kind) {
        (RunFilter::All, _) => false,
        (RunFilter::Running, RowKind::Task { task, .. }) => matches!(
            task.state,
            TaskState::Preparing
                | TaskState::Working
                | TaskState::Proof
                | TaskState::Check
                | TaskState::Review
                | TaskState::MergeQueue
        ),
        (RunFilter::Running, RowKind::AgentRound { round, .. }) => round.ended_at.is_none(),
        (RunFilter::Running, RowKind::Scout { scout, .. }) => {
            matches!(scout.state, ScoutState::Starting | ScoutState::Working)
        }
        (RunFilter::Blocked, RowKind::Task { task, .. }) => task.state == TaskState::Blocked,
        (RunFilter::Runtime(runtime), RowKind::Task { task, .. }) => task.route.runtime == runtime,
        (RunFilter::Runtime(runtime), RowKind::AgentRound { round, .. }) => {
            round.info.route.runtime == runtime
        }
        (RunFilter::Runtime(runtime), RowKind::Scout { scout, .. }) => {
            scout.route.runtime == runtime
        }
        _ => false,
    }
}

fn filter_children<'a>(
    children: Vec<Node<'a>>,
    inherit: Inherit,
    filters: &Filters<'_>,
) -> Vec<Node<'a>> {
    children
        .into_iter()
        .filter_map(|child| filter_node(child, inherit, filters))
        .collect()
}

/// Keeps a node that passes both filters itself (by its own match or an ancestor's),
/// or that leads to one that does; the rest is dropped (decision 21).
fn filter_node<'a>(
    mut node: Node<'a>,
    inherit: Inherit,
    filters: &Filters<'_>,
) -> Option<Node<'a>> {
    if !filters.active() {
        return Some(node);
    }
    let text_own = text_matches(&node, filters.text);
    let run_own = run_filter_matches(&node.row.kind, filters.run);
    let text_ok = filters.text.is_empty() || inherit.text || text_own;
    let run_ok = filters.run == RunFilter::All || inherit.run || run_own;
    let below = Inherit {
        text: inherit.text || text_own,
        run: inherit.run || run_own,
    };
    node.children = filter_children(std::mem::take(&mut node.children), below, filters);
    // Sub-agents follow their node through the `f` filter; the text filter shows all of
    // them below a match and only the matching branches otherwise.
    node.subagents = node.subagents.take().and_then(|mut subagents| {
        subagents.show_all = text_ok;
        let any = text_ok
            || (subagents.forest.iter())
                .any(|branch| subagent_branch_matches(branch, filters.text));
        (run_ok && any).then_some(subagents)
    });
    let kept = (text_ok && run_ok) || !node.children.is_empty() || node.subagents.is_some();
    kept.then_some(node)
}

struct Walk<'s, 'f> {
    state: &'s TreeState,
    filters: &'s Filters<'f>,
}

/// Appends `node`'s row and everything below it. `has_later` is the node's own guide
/// bit, `None` for the root, which carries no guides.
fn emit<'a>(
    rows: &mut Vec<Row<'a>>,
    mut node: Node<'a>,
    walk: &Walk<'_, '_>,
    ancestors: &mut Vec<bool>,
    has_later: Option<bool>,
) {
    let depth = u16::try_from(ancestors.len()).unwrap_or(u16::MAX);
    let depth = if has_later.is_some() {
        depth.saturating_add(1)
    } else {
        depth
    };
    node.row.depth = depth;
    if let Some(has_later) = has_later {
        node.row.guides = guide_prefix(ancestors, has_later);
    }
    let folded = !walk.filters.active() && walk.state.is_collapsed(&node.row.key);
    rows.push(node.row);
    if folded {
        return;
    }
    if let Some(bit) = has_later {
        ancestors.push(bit);
    }
    let shown_subagents = node.subagents.as_ref().map_or(0, |subagents| {
        (subagents.forest.iter())
            .filter(|branch| {
                subagents.show_all || subagent_branch_matches(branch, walk.filters.text)
            })
            .count()
    });
    let count = node.children.len();
    for (index, child) in node.children.into_iter().enumerate() {
        let later = index + 1 < count || shown_subagents > 0;
        emit(rows, child, walk, ancestors, Some(later));
    }
    if let Some(subagents) = &node.subagents {
        emit_subagents(
            rows,
            &SubagentWalk {
                window: subagents.window,
                filter: walk.filters.text,
                show_all: subagents.show_all,
            },
            &subagents.forest,
            ancestors,
            depth.saturating_add(1),
            false,
        );
    }
    if has_later.is_some() {
        ancestors.pop();
    }
}
