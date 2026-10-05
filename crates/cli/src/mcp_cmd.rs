//! `anthrex mcp` (hidden): the arguments the daemon's headless launcher passes
//! (`daemon::headless::argv::mcp_args`), turned into `mcp::McpOptions`.

use clap::{Args, ValueEnum};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct McpArgs {
    #[arg(long, value_enum)]
    role: RoleArg,
    /// Required for every role but `scout`: a repository-level scout belongs to no run
    /// (M8b decision 15).
    #[arg(long = "run", required_if_eq_any([
        ("role", "worker"), ("role", "reviewer"), ("role", "orchestrator"),
        ("role", "planner"), ("role", "racer"), ("role", "test_writer"),
        ("role", "brainstormer"), ("role", "doc_reviewer"),
    ]))]
    run_id: Option<String>,
    #[arg(long = "task")]
    task_id: Option<String>,
    #[arg(long = "scout")]
    scout_id: Option<String>,
    /// A sub-planner's epic (milestone 9): required for `planner`, refused otherwise.
    #[arg(long = "epic")]
    epic: Option<String>,
    /// A chained orchestrator's chain (milestone 9.3): refused for every other role.
    #[arg(long = "chain")]
    chain: Option<String>,
    /// A racer's lane (milestone 9.5): required for `racer`, refused otherwise.
    #[arg(long = "lane", value_enum)]
    lane: Option<LaneArg>,
    /// A design agent's label (milestone 9.6 ruling T1-O3): accepted only with
    /// `brainstormer` and `doc_reviewer`, and refused otherwise; the server never sends
    /// it on. It names the session in its argv.
    #[arg(long = "agent-label")]
    agent_label: Option<String>,
    #[arg(long = "window")]
    window_id: u32,
    /// Defaults to the daemon's usual socket
    #[arg(long)]
    socket: Option<PathBuf>,
}

/// `--role`. `orchestrator` and `planner` serve milestone 9's tools (decision 15);
/// `scout` (milestone 8b) serves `submit_scout_report`, for a scout or a research
/// task; `racer` and `test_writer` (milestone 9.5) serve the worker's tools;
/// `brainstormer` and `doc_reviewer` (milestone 9.6) serve the design flow's. There is
/// no `decider`: a decider never runs `anthrex mcp` (`daemon::headless::argv::mcp_args`
/// refuses one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RoleArg {
    Worker,
    Reviewer,
    Orchestrator,
    Scout,
    Planner,
    Racer,
    /// Spelt as `proto::AgentRole` serializes it, not clap's default `test-writer`.
    #[value(name = "test_writer")]
    TestWriter,
    Brainstormer,
    #[value(name = "doc_reviewer")]
    DocReviewer,
}

/// `--lane`: `a` or `b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum LaneArg {
    A,
    B,
}

impl From<LaneArg> for proto::RaceLane {
    fn from(l: LaneArg) -> Self {
        match l {
            LaneArg::A => proto::RaceLane::A,
            LaneArg::B => proto::RaceLane::B,
        }
    }
}

impl From<RoleArg> for proto::AgentRole {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::Worker => proto::AgentRole::Worker,
            RoleArg::Reviewer => proto::AgentRole::Reviewer,
            RoleArg::Orchestrator => proto::AgentRole::Orchestrator,
            RoleArg::Scout => proto::AgentRole::Scout,
            RoleArg::Planner => proto::AgentRole::Planner,
            RoleArg::Racer => proto::AgentRole::Racer,
            RoleArg::TestWriter => proto::AgentRole::TestWriter,
            RoleArg::Brainstormer => proto::AgentRole::Brainstormer,
            RoleArg::DocReviewer => proto::AgentRole::DocReviewer,
        }
    }
}

impl McpArgs {
    /// `default_socket` is used when `--socket` was not given. The checks clap cannot
    /// state per role are a usage error (exit code 2), as clap's own are: `--epic` is
    /// required for `planner` and refused for every other role; `--chain` is refused for
    /// every role but `orchestrator` (milestone 9.3); `--lane` is required for `racer`
    /// and refused for every other role (milestone 9.5); `--agent-label` is refused for
    /// every role but `brainstormer` and `doc_reviewer` (milestone 9.6); `--role scout`
    /// takes exactly one of `--scout` and `--task` (milestone 9 decision 35, defect 19).
    pub fn into_options(self, default_socket: PathBuf) -> Result<mcp::McpOptions, clap::Error> {
        use clap::error::ErrorKind;
        let usage = |kind, text: &str| Err(clap::Error::raw(kind, format!("{text}\n")));
        match (self.role, &self.epic) {
            (RoleArg::Planner, None) => {
                return usage(
                    ErrorKind::MissingRequiredArgument,
                    "--role planner requires --epic <e>",
                );
            }
            (RoleArg::Planner, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return usage(
                    ErrorKind::ArgumentConflict,
                    "--epic <e> is accepted only with --role planner",
                );
            }
        }
        if self.role != RoleArg::Orchestrator && self.chain.is_some() {
            return usage(
                ErrorKind::ArgumentConflict,
                "--chain <c> is accepted only with --role orchestrator",
            );
        }
        match (self.role, self.lane) {
            (RoleArg::Racer, None) => {
                return usage(
                    ErrorKind::MissingRequiredArgument,
                    "--role racer requires --lane <a|b>",
                );
            }
            (RoleArg::Racer, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return usage(
                    ErrorKind::ArgumentConflict,
                    "--lane <a|b> is accepted only with --role racer",
                );
            }
        }
        let design = matches!(self.role, RoleArg::Brainstormer | RoleArg::DocReviewer);
        if !design && self.agent_label.is_some() {
            return usage(
                ErrorKind::ArgumentConflict,
                "--agent-label <text> is accepted only with --role brainstormer or doc_reviewer",
            );
        }
        if self.role == RoleArg::Scout && self.scout_id.is_some() == self.task_id.is_some() {
            return usage(
                ErrorKind::ArgumentConflict,
                "--role scout takes exactly one of --scout <id> or --task <t>",
            );
        }
        Ok(mcp::McpOptions {
            role: self.role.into(),
            run_id: self.run_id.unwrap_or_default(),
            task_id: self.task_id,
            scout_id: self.scout_id,
            epic: self.epic,
            chain: self.chain,
            lane: self.lane.map(Into::into),
            window_id: self.window_id,
            socket: self.socket.unwrap_or(default_socket),
        })
    }
}

#[cfg(test)]
#[path = "mcp_cmd_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "mcp_cmd_design_tests.rs"]
mod design_tests;
