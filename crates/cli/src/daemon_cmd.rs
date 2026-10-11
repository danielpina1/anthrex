//! `anthrex daemon start|stop|status` (moved out of `main.rs` by task M9.11.27).

use super::DaemonAction;
use crate::client;
use proto::ClientMsg;
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::UnixStream;

/// How long the stopped daemon's runtime waits for its blocking tasks (decision 30). A git
/// or file call stuck on the blocking pool would otherwise keep the process alive after
/// `daemon stop` has returned, since dropping a runtime waits for every blocking task.
const BLOCKING_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// `daemon start --foreground` (decision 30): the daemon on a runtime of its own, dropped
/// with `shutdown_timeout(BLOCKING_SHUTDOWN_GRACE)` once `daemon::run` returns, so a stuck
/// blocking task can delay the exit by that grace and no more. The lifetime lock is
/// `run`'s own local and is released when `run` returns, before this drop, so `daemon
/// stop`'s `wait_released` is unaffected.
pub(super) fn foreground(socket: PathBuf) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(daemon::run(daemon::DaemonOptions {
        socket_path: socket,
        data_dir: proto::paths::data_dir(),
        config_path: proto::paths::config_path(),
        lock_wait: daemon::LOCK_WAIT,
    }));
    runtime.shutdown_timeout(BLOCKING_SHUTDOWN_GRACE);
    result
}

pub(super) async fn daemon_command(action: DaemonAction, socket: PathBuf) -> anyhow::Result<()> {
    match action {
        DaemonAction::Start { foreground: true } => {
            unreachable!("main runs the foreground daemon on its own runtime (decision 30)")
        }
        DaemonAction::Start { foreground: false } => {
            tui::spawn::ensure_daemon(&std::env::current_exe()?, &socket).await?;
            println!("daemon running on {}", socket.display());
            Ok(())
        }
        DaemonAction::Stop => {
            let mut c = client::CliClient::connect(&socket)
                .await
                .map_err(|_| anyhow::anyhow!("no daemon is running"))?;
            c.send(ClientMsg::Shutdown).await?;
            c.wait_close().await;
            // Decision 27: only report the daemon stopped once its lifetime lock is
            // free, so an `anthrex` right after this never races a daemon that is still
            // tearing down.
            let lock_path = proto::paths::lock_path();
            let released = tokio::task::spawn_blocking(move || {
                daemon::lockfile::wait_released(&lock_path, Duration::from_secs(10))
            })
            .await?;
            if !released {
                anyhow::bail!(
                    "the daemon did not exit within 10 s; see {}",
                    proto::paths::log_path().display()
                );
            }
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
