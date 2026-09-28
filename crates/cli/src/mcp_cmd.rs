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
        ("role", "planner"),
    ]))]
    run_id: Option<String>,
    #[arg(long = "task")]
    task_id: Option<String>,
    #[arg(long = "scout")]
    scout_id: Option<String>,
    #[arg(long = "window")]
    window_id: u32,
    /// Defaults to the daemon's usual socket
    #[arg(long)]
    socket: Option<PathBuf>,
}

/// `--role`. `orchestrator` and `planner` are accepted because the launcher can build
/// them; they serve no tools until milestone 9's M9.11. `scout` (milestone 8b) serves
/// `submit_scout_report`. There is no `decider`: a decider never runs `anthrex mcp`
/// (`daemon::headless::argv::mcp_args` refuses one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RoleArg {
    Worker,
    Reviewer,
    Orchestrator,
    Scout,
    Planner,
}

impl From<RoleArg> for proto::AgentRole {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::Worker => proto::AgentRole::Worker,
            RoleArg::Reviewer => proto::AgentRole::Reviewer,
            RoleArg::Orchestrator => proto::AgentRole::Orchestrator,
            RoleArg::Scout => proto::AgentRole::Scout,
            RoleArg::Planner => proto::AgentRole::Planner,
        }
    }
}

impl McpArgs {
    /// `default_socket` is used when `--socket` was not given.
    pub fn into_options(self, default_socket: PathBuf) -> mcp::McpOptions {
        mcp::McpOptions {
            role: self.role.into(),
            run_id: self.run_id.unwrap_or_default(),
            task_id: self.task_id,
            scout_id: self.scout_id,
            window_id: self.window_id,
            socket: self.socket.unwrap_or(default_socket),
        }
    }
}

#[cfg(test)]
#[path = "mcp_cmd_tests.rs"]
mod tests;
