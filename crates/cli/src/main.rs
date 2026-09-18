mod client;
mod spawn;

use clap::{Parser, Subcommand, ValueEnum};
use proto::{ClientMsg, DaemonMsg, Runtime, WindowSpec};
use std::path::PathBuf;
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(name = "anthrex", version, about = "A terminal multiplexer for coding agents")]
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
        /// Create a git worktree on this branch (implemented in a later milestone)
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Initial prompt for claude or codex
        #[arg(long)]
        prompt: Option<String>,
    },
    /// List windows
    Ls,
    /// Kill a window's process (SIGTERM, then SIGKILL after 3 s)
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
    dir.canonicalize().map_err(|e| anyhow::anyhow!("cannot resolve directory {}: {e}", dir.display()))
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket: PathBuf = proto::paths::socket_path();
    match cli.command {
        None => attach(socket, resolve_dir(cli.dir)?, None).await,
        Some(Command::Attach { target }) => attach(socket, resolve_dir(cli.dir)?, target).await,
        Some(Command::Daemon { action }) => daemon_command(action, socket).await,
        Some(Command::New { runtime, name, worktree, model, prompt }) => {
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
            match c.request(ClientMsg::CreateWindow { spec, cols, rows }).await? {
                DaemonMsg::Created { window_id } => {
                    println!("{window_id}");
                    Ok(())
                }
                DaemonMsg::Error { message, .. } => anyhow::bail!(message),
                other => anyhow::bail!("unexpected reply: {other:?}"),
            }
        }
        Some(Command::Ls) => {
            let c = client::CliClient::connect(&socket).await?;
            print!("{}", client::format_table(&c.windows));
            Ok(())
        }
        Some(Command::Kill { target }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(c.request(ClientMsg::Kill { window_id: id }).await?)
        }
        Some(Command::Rm { target, worktree, force }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(c.request(ClientMsg::Remove { window_id: id, remove_worktree: worktree, force }).await?)
        }
    }
}

async fn attach(socket: PathBuf, dir: PathBuf, target: Option<String>) -> anyhow::Result<()> {
    spawn::ensure_daemon(&socket).await?;
    tui::run(tui::TuiOptions { socket_path: socket, default_dir: dir, focus: target }).await
}

async fn daemon_command(action: DaemonAction, socket: PathBuf) -> anyhow::Result<()> {
    match action {
        DaemonAction::Start { foreground: true } => {
            daemon::run(daemon::DaemonOptions { socket_path: socket, data_dir: proto::paths::data_dir() }).await
        }
        DaemonAction::Start { foreground: false } => {
            spawn::ensure_daemon(&socket).await?;
            println!("daemon running on {}", socket.display());
            Ok(())
        }
        DaemonAction::Stop => {
            let mut c = client::CliClient::connect(&socket).await.map_err(|_| anyhow::anyhow!("no daemon is running"))?;
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
