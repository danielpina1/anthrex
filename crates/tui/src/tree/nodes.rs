//! The tree's node keys and rows (`NodeKey`, `RowKind`, `Row`), moved out of `tree.rs`
//! unchanged per the 600-line rule (milestone 9.6 task 18).

use super::DisplayRound;
use super::RuntimeCounts;
use proto::{
    AgentRole, DesignAgentInfo, IdleOrchestrator, PlannerInfo, RoundInfo, RunInfo, ScoutInfo,
    StageInfo, Status, SubagentInfo, TaskInfo, WindowInfo,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKey {
    Project(PathBuf),
    Window(u32),
    Subagent {
        window_id: u32,
        id: String,
    },
    /// Milestone 8c: a run, both its project-tree node and its run view's root.
    Run(String),
    Planner {
        run: String,
        epic: String,
    },
    Scout {
        run: String,
        id: String,
    },
    /// Milestone 9.6 ruling T18-1: a design agent of the run's round (a brainstormer or
    /// a document reviewer), by its label, one node however many sessions it had.
    DesignAgent {
        run: String,
        label: String,
    },
    Task {
        run: String,
        id: String,
    },
    /// Milestone 9.1 decision 55: a stage of a `Multi` run, between the run and its tasks.
    Stage {
        run: String,
        n: u16,
    },
    /// Milestone 9.3 decision 32: a round's separator in a run of several rounds.
    Round {
        run: String,
        n: u32,
    },
    /// Milestone 9.3 decision 32: a project's idle orchestrator, by its chain id.
    Chain(String),
    /// `round` is the display round (milestone 8c decision 14), not `AgentRoundInfo.round`.
    /// `lane` is a racer's or lane reviewer's race lane (milestone 9.5 ruling T20-1):
    /// both lanes number their reviewers from 1, so the lane tells them apart.
    AgentRound {
        run: String,
        task: String,
        role: AgentRole,
        lane: Option<proto::RaceLane>,
        session: u32,
        round: u32,
    },
}

/// A plain window of a project. A project's shown runs are held apart, in
/// `ProjectGroup::runs`: their rows sit above the plain windows, and the windows they
/// own are not members at all (milestone 8c decisions 7 and 8).
pub enum ProjectChild<'a> {
    Window(&'a WindowInfo),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowKind<'a> {
    Project {
        root: &'a Path,
        name: String,
        status: Status,
        counts: RuntimeCounts,
        collapsed: bool,
    },
    Window {
        info: &'a WindowInfo,
        position: usize,
        has_subagents: bool,
        collapsed: bool,
    },
    Subagent {
        info: &'a SubagentInfo,
    },
    /// Milestone 8c: a run, a leaf of the project tree and the run view's root. The
    /// orchestrator's window, when listed, takes `position` (decision 10).
    Run {
        run: &'a RunInfo,
        orchestrator: Option<&'a WindowInfo>,
        position: Option<usize>,
    },
    // The run view's rows (built from task M8c.4 on).
    Planner {
        run: &'a RunInfo,
        planner: &'a PlannerInfo,
    },
    Scout {
        run: &'a RunInfo,
        scout: &'a ScoutInfo,
        window: Option<&'a WindowInfo>,
    },
    /// Milestone 9.6 ruling T18-1: a design agent and its listed window.
    DesignAgent {
        run: &'a RunInfo,
        agent: &'a DesignAgentInfo,
        window: Option<&'a WindowInfo>,
    },
    Task {
        run: &'a RunInfo,
        task: &'a TaskInfo,
    },
    /// Milestone 9.1 decision 55: a stage node of a `Multi` run.
    Stage {
        run: &'a RunInfo,
        stage: &'a StageInfo,
    },
    /// Milestone 9.3 decision 32: `round <r> · <goal head>`, above the round's stages.
    Round {
        run: &'a RunInfo,
        round: &'a RoundInfo,
    },
    /// Milestone 9.3 decision 32: an idle, unended chain and its listed window, in the
    /// window's place (`◌ orchestrator · idle · after <h4>`).
    IdleOrchestrator {
        idle: &'a IdleOrchestrator,
        window: &'a WindowInfo,
    },
    AgentRound {
        run: &'a RunInfo,
        task: &'a TaskInfo,
        round: DisplayRound<'a>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    pub key: NodeKey,
    /// The box-drawing prefix to draw before the row's own marker, two columns
    /// per level. Empty on project rows, which are roots.
    pub guides: String,
    /// How far below a root this row sits: 0 for a project, 1 for a window, 2
    /// for one of that window's sub-agents, and one more per nested level.
    ///
    /// Carried explicitly rather than recovered from `guides`, whose width per
    /// level is the sidebar's business alone: the graph overview's tiers must
    /// not move when the guide alphabet changes.
    pub depth: u16,
    pub kind: RowKind<'a>,
}
