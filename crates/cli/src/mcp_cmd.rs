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
    /// A sub-planner's epic (milestone 9): required for `planner`, refused otherwise.
    #[arg(long = "epic")]
    epic: Option<String>,
    /// A chained orchestrator's chain (milestone 9.3): refused for every other role.
    #[arg(long = "chain")]
    chain: Option<String>,
    #[arg(long = "window")]
    window_id: u32,
    /// Defaults to the daemon's usual socket
    #[arg(long)]
    socket: Option<PathBuf>,
}

/// `--role`. `orchestrator` and `planner` serve milestone 9's tools (decision 15);
/// `scout` (milestone 8b) serves `submit_scout_report`, for a scout or a research
/// task. There is no `decider`: a decider never runs `anthrex mcp`
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
    /// `default_socket` is used when `--socket` was not given. The checks clap cannot
    /// state per role are a usage error (exit code 2), as clap's own are: `--epic` is
    /// required for `planner` and refused for every other role; `--chain` is refused for
    /// every role but `orchestrator` (milestone 9.3); `--role scout` takes
    /// exactly one of `--scout` and `--task` (milestone 9 decision 35, defect 19).
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
            window_id: self.window_id,
            socket: self.socket.unwrap_or(default_socket),
        })
    }
}

#[cfg(test)]
#[path = "mcp_cmd_tests.rs"]
mod tests;
