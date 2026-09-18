mod client;
mod spawn;

use clap::{Parser, Subcommand, ValueEnum};
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, Runtime, WindowSpec, read_frame, write_frame};
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
    let dir = match cli.dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    }
    .canonicalize()?;
    match cli.command {
        None => {
            println!("anthrex {} — attach comes in a later task", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Command::Daemon { action }) => daemon_command(action, socket).await,
        Some(Command::New { runtime, name, worktree, model, prompt }) => {
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
            let stream = UnixStream::connect(&socket).await.map_err(|_| anyhow::anyhow!("no daemon is running"))?;
            let (mut rd, mut wr) = stream.into_split();
            write_frame(&mut wr, &ClientMsg::Hello { proto_version: PROTO_VERSION, client: ClientKind::Cli }).await?;
            let _welcome: Option<DaemonMsg> = read_frame(&mut rd).await?;
            write_frame(&mut wr, &ClientMsg::Shutdown).await?;
            // Drain until the daemon closes the connection.
            while let Ok(Some(_)) = read_frame::<_, DaemonMsg>(&mut rd).await {}
            println!("daemon stopped");
            Ok(())
        }
        DaemonAction::Status => match UnixStream::connect(&socket).await {
            Ok(stream) => {
                let (mut rd, mut wr) = stream.into_split();
                write_frame(&mut wr, &ClientMsg::Hello { proto_version: PROTO_VERSION, client: ClientKind::Cli }).await?;
                match read_frame::<_, DaemonMsg>(&mut rd).await? {
                    Some(DaemonMsg::Welcome { daemon_version, windows }) => {
                        println!("running: version {daemon_version}, {} window(s), socket {}", windows.len(), socket.display());
                    }
                    Some(DaemonMsg::Error { message, .. }) => println!("running but incompatible: {message}"),
                    _ => println!("running but did not answer the handshake"),
                }
                Ok(())
            }
            Err(_) => {
                println!("not running (socket {})", socket.display());
                Ok(())
            }
        },
    }
}
