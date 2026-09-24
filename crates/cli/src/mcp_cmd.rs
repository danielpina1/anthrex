//! `anthrex mcp` (hidden): the arguments the daemon's headless launcher passes
//! (`daemon::headless::argv::mcp_args`), turned into `mcp::McpOptions`.

use clap::{Args, ValueEnum};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct McpArgs {
    #[arg(long, value_enum)]
    role: RoleArg,
    #[arg(long = "run")]
    run_id: String,
    #[arg(long = "task")]
    task_id: Option<String>,
    #[arg(long = "window")]
    window_id: u32,
    /// Defaults to the daemon's usual socket
    #[arg(long)]
    socket: Option<PathBuf>,
}

/// `--role`. `orchestrator` is accepted because the launcher can build it; it serves no
/// tools until milestone 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RoleArg {
    Worker,
    Reviewer,
    Orchestrator,
}

impl From<RoleArg> for proto::AgentRole {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::Worker => proto::AgentRole::Worker,
            RoleArg::Reviewer => proto::AgentRole::Reviewer,
            RoleArg::Orchestrator => proto::AgentRole::Orchestrator,
        }
    }
}

impl McpArgs {
    /// `default_socket` is used when `--socket` was not given.
    pub fn into_options(self, default_socket: PathBuf) -> mcp::McpOptions {
        mcp::McpOptions {
            role: self.role.into(),
            run_id: self.run_id,
            task_id: self.task_id,
            window_id: self.window_id,
            socket: self.socket.unwrap_or(default_socket),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Cli, Command};
    use clap::Parser;
    use daemon::headless::McpTarget;
    use proto::AgentRole;
    use std::path::{Path, PathBuf};

    /// The argv the daemon's headless launcher builds is exactly what `anthrex mcp`
    /// parses, with and without a task; and `mcp` stays out of the help.
    #[test]
    fn mcp_parses_the_daemons_headless_argv_and_is_hidden() {
        let socket = Path::new("/tmp/anthrex-x/d.sock");
        for (role, task) in [
            (AgentRole::Worker, Some("t1")),
            (AgentRole::Reviewer, Some("t2")),
            (AgentRole::Orchestrator, None),
        ] {
            let target = McpTarget {
                role,
                run_id: "add-reset-3f9a".into(),
                task_id: task.map(String::from),
            };
            let mut argv = vec!["anthrex".to_string()];
            argv.extend(daemon::headless::argv::mcp_args(&target, 12, socket));
            match Cli::try_parse_from(&argv)
                .unwrap_or_else(|e| panic!("{argv:?}: {e}"))
                .command
            {
                Some(Command::Mcp(args)) => {
                    let opts = args.into_options(PathBuf::from("/tmp/unused.sock"));
                    assert_eq!(
                        opts,
                        mcp::McpOptions {
                            role,
                            run_id: "add-reset-3f9a".into(),
                            task_id: task.map(String::from),
                            window_id: 12,
                            socket: socket.to_path_buf(),
                        }
                    );
                }
                other => panic!("expected Mcp, got {other:?}"),
            }
        }

        let help = Cli::try_parse_from(["anthrex", "--help"])
            .err()
            .expect("help exits")
            .to_string();
        assert!(!help.contains("mcp"), "mcp must be hidden: {help}");
    }

    #[test]
    fn mcp_socket_defaults_to_the_daemons() {
        let argv = [
            "anthrex", "mcp", "--role", "worker", "--run", "r", "--window", "1",
        ];
        match Cli::try_parse_from(argv).unwrap().command {
            Some(Command::Mcp(args)) => {
                let opts = args.into_options(PathBuf::from("/tmp/default.sock"));
                assert_eq!(opts.socket, PathBuf::from("/tmp/default.sock"));
                assert_eq!(opts.task_id, None);
            }
            other => panic!("expected Mcp, got {other:?}"),
        }
    }
}
