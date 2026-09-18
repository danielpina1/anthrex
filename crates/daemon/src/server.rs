//! Accepts client connections and speaks the protocol from spec section 4.

use crate::manager::WindowManager;
use crate::window::Attachment;
use bytes::Bytes;
use proto::{ClientMsg, DaemonMsg, PROTO_VERSION, read_frame, write_frame};
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Runs until `shutdown` is cancelled. Each connection gets its own task.
pub async fn serve(
    listener: UnixListener,
    manager: Arc<WindowManager>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            accepted = listener.accept() => {
                // An accept error must never end `serve`: `run` would then kill every
                // agent the daemon owns. EMFILE is entirely reachable (macOS defaults to
                // 256 descriptors), and it clears as soon as a client disconnects, so
                // back off briefly and keep listening.
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        tracing::warn!(error = %e, "accept failed; still listening");
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        continue;
                    }
                };
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
    DaemonMsg::Error {
        request: request.to_string(),
        message: message.into(),
    }
}

fn ack_or_error(request: &str, result: anyhow::Result<()>) -> DaemonMsg {
    match result {
        Ok(()) => DaemonMsg::Ack {
            request: request.to_string(),
        },
        Err(e) => error(request, e.to_string()),
    }
}

/// Owns one client's view, including while its initial snapshot is being sent.
struct Subscription {
    window_id: u32,
    manager: Arc<WindowManager>,
    task: Option<JoinHandle<()>>,
}

impl Subscription {
    async fn stop(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
        self.manager.unfocus(self.window_id);
    }
}

async fn handle_client(
    stream: UnixStream,
    manager: Arc<WindowManager>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let (mut rd, mut wr) = stream.into_split();

    let Some(ClientMsg::Hello {
        proto_version,
        client,
    }) = read_frame::<_, ClientMsg>(&mut rd).await?
    else {
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
        &DaemonMsg::Welcome {
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            windows: manager.list(),
        },
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
            if changes_out
                .send(DaemonMsg::WindowsChanged { windows })
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let mut subscription: Option<Subscription> = None;
    let mut connection_error = None;
    loop {
        let msg = tokio::select! {
            _ = shutdown.cancelled() => {
                // Bye is best-effort: an awaited send would block forever on a stalled
                // client whose full channel nobody is draining, delaying shutdown cleanup.
                let _ = out_tx.try_send(DaemonMsg::Bye { reason: "daemon shutting down".into() });
                break;
            }
            frame = read_frame::<_, ClientMsg>(&mut rd) => match frame {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(error) => {
                    connection_error = Some(error);
                    break;
                }
            },
        };

        let reply = match msg {
            ClientMsg::Hello { .. } => Some(error("hello", "already greeted")),
            ClientMsg::ListWindows => Some(DaemonMsg::WindowsChanged {
                windows: manager.list(),
            }),
            ClientMsg::CreateWindow { spec, cols, rows } => {
                Some(match manager.create(spec, cols, rows) {
                    Ok(info) => DaemonMsg::Created { window_id: info.id },
                    Err(e) => error("create", e.to_string()),
                })
            }
            ClientMsg::Subscribe {
                window_id,
                cols,
                rows,
            } => {
                // `abort` only takes effect at the task's next yield point, so a forwarder
                // that is mid-`send` could still queue an Output behind the new Snapshot
                // and have the client apply that chunk twice. Wait for it to be gone.
                if let Some(previous) = subscription.take() {
                    previous.stop().await;
                }
                match manager
                    .resize(window_id, cols, rows)
                    .and_then(|_| manager.attach(window_id))
                {
                    Ok(att) => {
                        manager.focus(window_id);
                        subscription = Some(Subscription {
                            window_id,
                            manager: manager.clone(),
                            task: None,
                        });
                        // Snapshot must be queued before the forwarder can queue live output.
                        let snapshot = DaemonMsg::Snapshot {
                            window_id,
                            cols: att.cols,
                            rows: att.rows,
                            bytes: att.snapshot,
                        };
                        if out_tx.send(snapshot).await.is_err() {
                            break;
                        }
                        subscription.as_mut().expect("view acquired above").task =
                            Some(tokio::spawn(forward_output(
                                window_id,
                                att.output,
                                out_tx.clone(),
                                manager.clone(),
                            )));
                        None
                    }
                    Err(e) => Some(error("subscribe", e.to_string())),
                }
            }
            ClientMsg::Unsubscribe => {
                if let Some(previous) = subscription.take() {
                    previous.stop().await;
                }
                Some(DaemonMsg::Ack {
                    request: "unsubscribe".into(),
                })
            }
            ClientMsg::Input { window_id, bytes } => manager
                .write_input(window_id, &bytes)
                .err()
                .map(|e| error("input", e.to_string())),
            ClientMsg::Resize {
                window_id,
                cols,
                rows,
            } => manager
                .resize(window_id, cols, rows)
                .err()
                .map(|e| error("resize", e.to_string())),
            ClientMsg::Kill { window_id } => Some(ack_or_error("kill", manager.kill(window_id))),
            ClientMsg::Remove { window_id, .. } => {
                Some(ack_or_error("remove", manager.remove(window_id)))
            }
            ClientMsg::Rename { window_id, name } => {
                Some(ack_or_error("rename", manager.rename(window_id, name)))
            }
            ClientMsg::Restart { .. } => Some(error(
                "restart",
                "restart is not supported by this daemon version",
            )),
            ClientMsg::HookEvent {
                window_id,
                source,
                payload,
            } => Some(ack_or_error(
                "hook",
                manager.handle_hook(window_id, source, &payload),
            )),
            ClientMsg::Shutdown => {
                tracing::info!("shutdown requested by client");
                shutdown.cancel();
                None
            }
        };
        if let Some(reply) = reply
            && out_tx.send(reply).await.is_err()
        {
            break;
        }
    }

    if let Some(previous) = subscription.take() {
        previous.stop().await;
    }
    changes_task.abort();
    drop(out_tx);
    let _ = writer.await;
    tracing::debug!("client disconnected");
    match connection_error {
        Some(error) => Err(error.into()),
        None => Ok(()),
    }
}

/// Copies live PTY output to the client; a lagging client gets a fresh snapshot instead of the gap.
async fn forward_output(
    window_id: u32,
    output: broadcast::Receiver<Bytes>,
    out: mpsc::Sender<DaemonMsg>,
    manager: Arc<WindowManager>,
) {
    forward_output_from(window_id, output, out, move || manager.attach(window_id)).await
}

/// The forwarding loop, with re-attaching factored out so tests can drive it directly.
async fn forward_output_from(
    window_id: u32,
    mut output: broadcast::Receiver<Bytes>,
    out: mpsc::Sender<DaemonMsg>,
    reattach: impl Fn() -> anyhow::Result<Attachment> + Send,
) {
    loop {
        match output.recv().await {
            Ok(chunk) => {
                if out
                    .send(DaemonMsg::Output {
                        window_id,
                        bytes: chunk.to_vec(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!(window_id, missed = n, "client lagged; resending snapshot");
                // A lagged receiver resumes at the OLDEST RETAINED chunk, every one of
                // which the fresh snapshot already contains. Continuing with it would
                // replay up to a full channel's worth of output on top of the snapshot,
                // so the receiver is replaced by the one `attach` takes under the same
                // lock as the snapshot.
                match reattach() {
                    Ok(att) => {
                        output = att.output;
                        let snapshot = DaemonMsg::Snapshot {
                            window_id,
                            cols: att.cols,
                            rows: att.rows,
                            bytes: att.snapshot,
                        };
                        if out.send(snapshot).await.is_err() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::LaunchPlan;
    use crate::window::Window;
    use std::time::{Duration, Instant};

    /// I1: after `Lagged`, the forwarder must not replay the chunks the fresh snapshot
    /// already contains. The mirror a client would build from the forwarded messages has
    /// to match the daemon's own screen exactly.
    ///
    /// The child is long finished before anything is drained, so the snapshot taken on
    /// the lag provably contains every chunk still retained by the broadcast channel -
    /// continuing with the old receiver replays exactly those, and the twelve printed
    /// lines all fit on one screen, so a duplicate cannot scroll out of sight.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_lagged_subscriber_is_resynced_without_replaying_retained_chunks() {
        let plan = LaunchPlan {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "i=1; while [ $i -le 12 ]; do echo line-$i; sleep 0.03; i=$((i+1)); done".into(),
            ],
            cwd: std::env::temp_dir(),
            env: vec![("TERM".into(), "xterm-256color".into())],
        };
        let (events, _events_rx) = mpsc::unbounded_channel();
        // `Window` is Send but not Sync, so the mutex is what lets the forwarder task
        // and the test share it. Capacity 2: the forwarder lags as soon as it waits.
        let window = Arc::new(std::sync::Mutex::new(
            Window::spawn_with_output_capacity(1, &plan, 80, 24, events, 2).unwrap(),
        ));

        let first = window.lock().unwrap().attach();
        let mut mirror = vt100::Parser::new(first.rows, first.cols, 0);
        mirror.process(&first.snapshot);

        // A one-slot outgoing channel that nobody drains: exactly the shape of a client
        // that cannot keep up.
        let (out, mut out_rx) = mpsc::channel::<DaemonMsg>(1);
        let forwarder = {
            let window = Arc::clone(&window);
            tokio::spawn(forward_output_from(1, first.output, out, move || {
                Ok(window.lock().unwrap().attach())
            }))
        };

        // Long enough for every line to be printed and the child to exit.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let expected = window.lock().unwrap().screen_text();
        assert!(
            expected.contains("line-12"),
            "the child did not finish: {expected:?}"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut lagged = false;
        loop {
            match tokio::time::timeout(Duration::from_millis(500), out_rx.recv()).await {
                Ok(Some(DaemonMsg::Snapshot {
                    cols, rows, bytes, ..
                })) => {
                    lagged = true;
                    mirror = vt100::Parser::new(rows, cols, 0);
                    mirror.process(&bytes);
                }
                Ok(Some(DaemonMsg::Output { bytes, .. })) => mirror.process(&bytes),
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => break, // quiet for 500 ms: nothing more is coming
            }
            assert!(Instant::now() < deadline, "the forwarder never went quiet");
        }
        forwarder.abort();

        assert!(
            lagged,
            "the subscriber never lagged; the test did not exercise the recovery path"
        );
        assert_eq!(
            mirror.screen().contents(),
            expected,
            "the mirror diverged from the daemon's screen (chunks replayed on top of a snapshot that held them)"
        );
    }
}
