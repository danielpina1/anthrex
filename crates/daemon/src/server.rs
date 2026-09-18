//! Accepts client connections and speaks the protocol from spec section 4.

use crate::manager::WindowManager;
use bytes::Bytes;
use proto::{ClientMsg, DaemonMsg, PROTO_VERSION, read_frame, write_frame};
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Runs until `shutdown` is cancelled. Each connection gets its own task.
pub async fn serve(listener: UnixListener, manager: Arc<WindowManager>, shutdown: CancellationToken) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let manager = manager.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, manager, shutdown).await {
                        tracing::warn!(error = %e, "client connection ended with error");
                    }
                });
            }
        }
    }
}

fn error(request: &str, message: impl Into<String>) -> DaemonMsg {
    DaemonMsg::Error { request: request.to_string(), message: message.into() }
}

fn ack_or_error(request: &str, result: anyhow::Result<()>) -> DaemonMsg {
    match result {
        Ok(()) => DaemonMsg::Ack { request: request.to_string() },
        Err(e) => error(request, e.to_string()),
    }
}

async fn handle_client(stream: UnixStream, manager: Arc<WindowManager>, shutdown: CancellationToken) -> anyhow::Result<()> {
    let (mut rd, mut wr) = stream.into_split();

    let Some(ClientMsg::Hello { proto_version, client }) = read_frame::<_, ClientMsg>(&mut rd).await? else {
        return Ok(()); // EOF or a client that skipped the handshake: drop silently.
    };
    if proto_version != PROTO_VERSION {
        let message = format!(
            "protocol version mismatch (client {proto_version}, daemon {PROTO_VERSION}); run `anthrex daemon stop` and try again"
        );
        write_frame(&mut wr, &error("hello", message)).await?;
        return Ok(());
    }
    tracing::debug!(?client, "client connected");
    write_frame(
        &mut wr,
        &DaemonMsg::Welcome { daemon_version: env!("CARGO_PKG_VERSION").to_string(), windows: manager.list() },
    )
    .await?;

    // All outgoing traffic goes through one channel so the writer is never shared.
    let (out_tx, mut out_rx) = mpsc::channel::<DaemonMsg>(256);
    let writer: JoinHandle<()> = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if write_frame(&mut wr, &msg).await.is_err() {
                break;
            }
        }
    });

    let mut changes = manager.watch();
    let changes_out = out_tx.clone();
    let changes_task = tokio::spawn(async move {
        while changes.changed().await.is_ok() {
            let windows = changes.borrow_and_update().clone();
            if changes_out.send(DaemonMsg::WindowsChanged { windows }).await.is_err() {
                break;
            }
        }
    });

    let mut subscription: Option<JoinHandle<()>> = None;
    loop {
        let msg = tokio::select! {
            _ = shutdown.cancelled() => {
                // Bye is best-effort: an awaited send would block forever on a stalled
                // client whose full channel nobody is draining, delaying shutdown cleanup.
                let _ = out_tx.try_send(DaemonMsg::Bye { reason: "daemon shutting down".into() });
                break;
            }
            frame = read_frame::<_, ClientMsg>(&mut rd) => match frame? {
                Some(m) => m,
                None => break,
            },
        };

        let reply = match msg {
            ClientMsg::Hello { .. } => Some(error("hello", "already greeted")),
            ClientMsg::ListWindows => Some(DaemonMsg::WindowsChanged { windows: manager.list() }),
            ClientMsg::CreateWindow { spec, cols, rows } => Some(match manager.create(spec, cols, rows) {
                Ok(info) => DaemonMsg::Created { window_id: info.id },
                Err(e) => error("create", e.to_string()),
            }),
            ClientMsg::Subscribe { window_id, cols, rows } => {
                if let Some(task) = subscription.take() {
                    task.abort();
                }
                match manager.resize(window_id, cols, rows).and_then(|_| manager.attach(window_id)) {
                    Ok(att) => {
                        manager.focus(window_id);
                        // Snapshot must be queued before the forwarder can queue live output.
                        let snapshot = DaemonMsg::Snapshot { window_id, cols: att.cols, rows: att.rows, bytes: att.snapshot };
                        if out_tx.send(snapshot).await.is_err() {
                            break;
                        }
                        subscription = Some(tokio::spawn(forward_output(window_id, att.output, out_tx.clone(), manager.clone())));
                        None
                    }
                    Err(e) => Some(error("subscribe", e.to_string())),
                }
            }
            ClientMsg::Unsubscribe => {
                if let Some(task) = subscription.take() {
                    task.abort();
                }
                Some(DaemonMsg::Ack { request: "unsubscribe".into() })
            }
            ClientMsg::Input { window_id, bytes } => manager.write_input(window_id, &bytes).err().map(|e| error("input", e.to_string())),
            ClientMsg::Resize { window_id, cols, rows } => manager.resize(window_id, cols, rows).err().map(|e| error("resize", e.to_string())),
            ClientMsg::Kill { window_id } => Some(ack_or_error("kill", manager.kill(window_id))),
            ClientMsg::Remove { window_id, .. } => Some(ack_or_error("remove", manager.remove(window_id))),
            ClientMsg::Rename { window_id, name } => Some(ack_or_error("rename", manager.rename(window_id, name))),
            ClientMsg::Restart { .. } => Some(error("restart", "restart is not supported by this daemon version")),
            ClientMsg::HookEvent { .. } => Some(error("hook", "hook events are not supported by this daemon version")),
            ClientMsg::Shutdown => {
                tracing::info!("shutdown requested by client");
                shutdown.cancel();
                None
            }
        };
        if let Some(reply) = reply {
            if out_tx.send(reply).await.is_err() {
                break;
            }
        }
    }

    if let Some(task) = subscription.take() {
        task.abort();
    }
    changes_task.abort();
    drop(out_tx);
    let _ = writer.await;
    tracing::debug!("client disconnected");
    Ok(())
}

/// Copies live PTY output to the client; a lagging client gets a fresh snapshot instead of the gap.
async fn forward_output(
    window_id: u32,
    mut output: broadcast::Receiver<Bytes>,
    out: mpsc::Sender<DaemonMsg>,
    manager: Arc<WindowManager>,
) {
    loop {
        match output.recv().await {
            Ok(chunk) => {
                if out.send(DaemonMsg::Output { window_id, bytes: chunk.to_vec() }).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!(window_id, missed = n, "client lagged; resending snapshot");
                match manager.snapshot(window_id) {
                    Ok((bytes, cols, rows)) => {
                        if out.send(DaemonMsg::Snapshot { window_id, cols, rows, bytes }).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
