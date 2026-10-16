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

/// How long `daemon stop` waits, once the lifetime lock is free, for the daemon's process
/// to exit (its exit lock, `lockfile::hold_until_exit`): the runtime's grace, plus 3 s
/// for the process's own exit on a loaded machine.
const EXIT_WAIT: Duration = BLOCKING_SHUTDOWN_GRACE.saturating_add(Duration::from_secs(3));

/// `daemon start --foreground` (decision 30): the daemon on a runtime of its own, dropped
/// with `shutdown_timeout(BLOCKING_SHUTDOWN_GRACE)` once `daemon::run` returns, so a stuck
/// blocking task can delay the exit by that grace and no more. The lifetime lock is
/// `run`'s own local and is released when `run` returns, before this drop, so `daemon
/// stop`'s `wait_released` is unaffected.
pub(super) fn foreground(socket: PathBuf) -> anyhow::Result<()> {
    // The ax-leak-2 fix: held until this process exits, past the grace below, so
    // `daemon stop` returns only once nothing of this daemon can still write.
    let data_dir = proto::paths::data_dir();
    std::fs::create_dir_all(&data_dir)?;
    if !daemon::lockfile::hold_until_exit(&proto::paths::exit_lock_path())? {
        eprintln!(
            "anthrex: an earlier daemon is still exiting; `daemon stop` will not wait for this one's exit"
        );
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(daemon::run(daemon::DaemonOptions {
        socket_path: socket,
        data_dir,
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
            // The ax-leak-2 fix: the lifetime lock is free once `run` has returned, but
            // the process may still be finishing blocking tasks for its grace; it has
            // exited only once its exit lock is free.
            let exit_lock = proto::paths::exit_lock_path();
            let exited = tokio::task::spawn_blocking(move || {
                daemon::lockfile::wait_released(&exit_lock, EXIT_WAIT)
            })
            .await?;
            if !exited {
                anyhow::bail!(
                    "the daemon released its data directory but its process did not exit within {} s; see {}",
                    EXIT_WAIT.as_secs(),
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
