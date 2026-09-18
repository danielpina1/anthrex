//! Socket setup, logging, pid file, signals, and the top-level daemon loop. Spec section 3.7.

use crate::manager::WindowManager;
use crate::server;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

pub struct DaemonOptions {
    pub socket_path: PathBuf,
    pub data_dir: PathBuf,
}

/// Creates the socket directory (mode 0700) and removes a stale socket file.
/// Fails if a live daemon answers on the socket.
pub fn prepare_socket(path: &Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        // Best-effort: tighten the directory when we own it. A shared directory we don't
        // own (e.g. the socket lives directly under /tmp) can't be chmod'd by us, and that
        // is not fatal — it just means we could not harden a directory we didn't create.
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => anyhow::bail!("another daemon is already listening on {}", path.display()),
            Err(_) => std::fs::remove_file(path)?,
        }
    }
    Ok(())
}

fn init_logging(data_dir: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    let file = tracing_appender::rolling::never(data_dir, "daemon.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    let filter = tracing_subscriber::EnvFilter::try_from_env("ANTHREX_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(writer).init();
    guard
}

/// Runs the daemon in the current process until a signal or a client asks it to stop.
pub async fn run(opts: DaemonOptions) -> anyhow::Result<()> {
    std::fs::create_dir_all(&opts.data_dir)?;
    let _log_guard = init_logging(&opts.data_dir);
    prepare_socket(&opts.socket_path)?;
    let listener = UnixListener::bind(&opts.socket_path)?;
    let pid_path = opts.data_dir.join("daemon.pid");
    std::fs::write(&pid_path, std::process::id().to_string())?;
    tracing::info!(socket = %opts.socket_path.display(), pid = std::process::id(), "daemon started");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let (manager, mut events) = WindowManager::new(opts.socket_path.clone(), shell);
    let shutdown = CancellationToken::new();

    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });

    let ticker = manager.clone();
    let tick_token = shutdown.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick_token.cancelled() => break,
                _ = interval.tick() => ticker.tick(),
            }
        }
    });

    let signal_token = shutdown.clone();
    tokio::spawn(async move {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        tracing::info!("signal received, shutting down");
        signal_token.cancel();
    });

    server::serve(listener, manager.clone(), shutdown.clone()).await?;
    tracing::info!("stopping agents");
    manager.shutdown().await;
    let _ = std::fs::remove_file(&opts.socket_path);
    let _ = std::fs::remove_file(&pid_path);
    tracing::info!("daemon stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_socket_file_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("sub").join("d.sock");
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        std::fs::write(&sock, b"not a socket").unwrap();
        prepare_socket(&sock).unwrap();
        assert!(!sock.exists());
        let mode = std::fs::metadata(sock.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn live_socket_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let err = prepare_socket(&sock).unwrap_err();
        assert!(err.to_string().contains("already"));
        assert!(sock.exists());
    }
}
