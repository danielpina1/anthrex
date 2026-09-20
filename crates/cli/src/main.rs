mod client;
mod hook;
mod spawn;
mod tree_cmd;

use clap::{Parser, Subcommand, ValueEnum};
use proto::{ClientMsg, DaemonMsg, Runtime, WindowSpec};
use std::path::PathBuf;
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(
    name = "anthrex",
    version,
    about = "A terminal multiplexer for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Directory new agents start in (default: the current directory)
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Attach to the daemon (the default when no command is given)
    Attach {
        /// Window id or name to focus
        target: Option<String>,
    },
    /// Manage the background daemon that owns the agents
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Create a window and print its id
    New {
        #[arg(long, value_enum, default_value_t = RuntimeArg::Shell)]
        runtime: RuntimeArg,
        #[arg(long)]
        name: Option<String>,
        /// Create a git worktree on this branch (rejected for now; arrives with the worktree milestone)
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Initial prompt for claude or codex
        #[arg(long)]
        prompt: Option<String>,
    },
    /// List windows
    Ls {
        /// Print the window list as pretty JSON
        #[arg(long)]
        json: bool,
    },
    /// Print the project tree
    Tree {
        /// Show the project containing this directory
        #[arg(long)]
        project: Option<PathBuf>,
        /// Print the project tree as pretty JSON
        #[arg(long)]
        json: bool,
    },
    /// Kill a window's process group (SIGHUP, then SIGTERM, then SIGKILL)
    Kill { target: String },
    /// Kill and forget a window
    Rm {
        target: String,
        #[arg(long)]
        worktree: bool,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum RuntimeArg {
    Claude,
    Codex,
    Shell,
}

impl From<RuntimeArg> for Runtime {
    fn from(r: RuntimeArg) -> Self {
        match r {
            RuntimeArg::Claude => Runtime::Claude,
            RuntimeArg::Codex => Runtime::Codex,
            RuntimeArg::Shell => Runtime::Shell,
        }
    }
}

fn expect_ack(reply: DaemonMsg) -> anyhow::Result<()> {
    match reply {
        DaemonMsg::Ack { .. } => Ok(()),
        DaemonMsg::Error { message, .. } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// Resolves the working directory for a new window: the given `--dir`, or the current
/// directory when none was given, canonicalized either way.
fn resolve_dir(dir: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let dir = match dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    dir.canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve directory {}: {e}", dir.display()))
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon (detached unless --foreground)
    Start {
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the daemon and every agent it owns
    Stop,
    /// Show whether a daemon is running
    Status,
}

fn main() -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("hook")) {
        hook::run(std::env::args_os().skip(2).collect(), started);
        std::process::exit(0);
    }
    run_cli()
}

#[tokio::main]
async fn run_cli() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket: PathBuf = proto::paths::socket_path();
    match cli.command {
        None => attach(socket, resolve_dir(cli.dir)?, None).await,
        Some(Command::Attach { target }) => attach(socket, resolve_dir(cli.dir)?, target).await,
        Some(Command::Daemon { action }) => daemon_command(action, socket).await,
        Some(Command::New {
            runtime,
            name,
            worktree,
            model,
            prompt,
        }) => {
            if worktree.is_some() {
                // Silently ignoring it would show the branch in the title bar with no
                // worktree behind it, so the user would think the agent was isolated.
                anyhow::bail!(daemon::WORKTREE_UNSUPPORTED);
            }
            let dir = resolve_dir(cli.dir)?;
            spawn::ensure_daemon(&socket).await?;
            let mut c = client::CliClient::connect(&socket).await?;
            let spec = WindowSpec {
                name,
                runtime: runtime.into(),
                cwd: dir,
                worktree_branch: worktree,
                model,
                initial_prompt: prompt,
            };
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
            match c
                .request_with_timeout(
                    ClientMsg::CreateWindow { spec, cols, rows },
                    client::CREATE_WINDOW_REPLY_TIMEOUT,
                )
                .await?
            {
                DaemonMsg::Created { window_id } => {
                    println!("{window_id}");
                    Ok(())
                }
                DaemonMsg::Error { message, .. } => anyhow::bail!(message),
                other => anyhow::bail!("unexpected reply: {other:?}"),
            }
        }
        Some(Command::Ls { json }) => {
            let c = client::CliClient::connect(&socket).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&c.windows)?);
            } else {
                print!("{}", client::format_table(&c.windows));
            }
            Ok(())
        }
        Some(Command::Tree { project, json }) => {
            let c = client::CliClient::connect(&socket).await?;
            let requested = project.map(|path| resolve_dir(Some(path))).transpose()?;
            let normalized = match requested.as_ref() {
                Some(path) => Some(daemon::project::resolve_roots(path.clone()).await.project),
                None => None,
            };
            let project = requested
                .as_deref()
                .zip(normalized.as_deref())
                .map(|(requested, normalized)| tree_cmd::ProjectQuery::new(requested, normalized));
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&tree_cmd::tree_json(&c.windows, project))?
                );
            } else {
                print!("{}", tree_cmd::tree_text(&c.windows, project));
            }
            Ok(())
        }
        Some(Command::Kill { target }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(c.request(ClientMsg::Kill { window_id: id }).await?)
        }
        Some(Command::Rm {
            target,
            worktree,
            force,
        }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(
                c.request(ClientMsg::Remove {
                    window_id: id,
                    remove_worktree: worktree,
                    force,
                })
                .await?,
            )
        }
    }
}

async fn attach(socket: PathBuf, dir: PathBuf, target: Option<String>) -> anyhow::Result<()> {
    spawn::ensure_daemon(&socket).await?;
    tui::run(tui::TuiOptions {
        socket_path: socket,
        default_dir: dir,
        focus: target,
    })
    .await
}

async fn daemon_command(action: DaemonAction, socket: PathBuf) -> anyhow::Result<()> {
    match action {
        DaemonAction::Start { foreground: true } => {
            daemon::run(daemon::DaemonOptions {
                socket_path: socket,
                data_dir: proto::paths::data_dir(),
            })
            .await
        }
        DaemonAction::Start { foreground: false } => {
            spawn::ensure_daemon(&socket).await?;
            println!("daemon running on {}", socket.display());
            Ok(())
        }
        DaemonAction::Stop => {
            let mut c = client::CliClient::connect(&socket)
                .await
                .map_err(|_| anyhow::anyhow!("no daemon is running"))?;
            c.send(ClientMsg::Shutdown).await?;
            c.wait_close().await;
            println!("daemon stopped");
            Ok(())
        }
        DaemonAction::Status => match UnixStream::connect(&socket).await {
            Ok(_) => match client::CliClient::connect(&socket).await {
                Ok(c) => {
                    println!(
                        "running: version {}, {} window(s), socket {}",
                        c.daemon_version,
                        c.windows.len(),
                        socket.display()
                    );
                    Ok(())
                }
                Err(e) => {
                    println!("running but incompatible: {e}");
                    Ok(())
                }
            },
            Err(_) => {
                println!("not running (socket {})", socket.display());
                Ok(())
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn tree_accepts_optional_project_and_json_flags() {
        for args in [
            vec!["anthrex", "tree"],
            vec!["anthrex", "tree", "--project", "/r/shop/src"],
            vec!["anthrex", "tree", "--json"],
            vec!["anthrex", "tree", "--project", "/r/shop/src", "--json"],
        ] {
            assert!(Cli::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn tree_help_describes_command_and_both_flags() {
        let error = Cli::try_parse_from(["anthrex", "tree", "--help"])
            .err()
            .expect("help exits instead of parsing a command");
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        let help = error.to_string();
        assert!(help.contains("Print the project tree"));
        assert!(help.contains("--project <PROJECT>"));
        assert!(help.contains("--json"));
    }
}
