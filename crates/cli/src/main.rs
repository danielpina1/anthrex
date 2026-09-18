mod spawn;

use clap::{Parser, Subcommand};
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, read_frame, write_frame};
use std::path::PathBuf;
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(name = "anthrex", version, about = "A terminal multiplexer for coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Manage the background daemon that owns the agents
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
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
        None => {
            println!("anthrex {} — attach comes in a later task", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Command::Daemon { action }) => daemon_command(action, socket).await,
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
