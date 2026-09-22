//! Accepts client connections and speaks the protocol from spec section 4.

mod conversation;
mod requests;

use crate::git::GitRegistry;
use crate::manager::WindowManager;
use crate::window::Attachment;
use bytes::Bytes;
use proto::messages::request;
use proto::{ClientMsg, DaemonMsg, GitState, PROTO_VERSION, read_frame, write_frame};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Runs until `shutdown` is cancelled. Each connection gets its own task.
///
/// Owns the [`GitRegistry`] for the whole daemon: one registry, watching and probing
/// whatever roots the connected clients' windows reference, and one broadcast channel
/// that turns its publications into [`DaemonMsg::Git`] for every attached client. The
/// manager itself holds no git state and takes no git-related lock (AGENTS.md hard rule
/// 2).
///
/// Where registration and unregistration happen is not one rule but two, and the
/// difference matters. A plain create and a plain `Remove { remove_worktree: false }`
/// register or unregister from `server::requests`, in `handle_client`'s own task, always
/// after the call that changed the window table has already returned — see
/// [`requests::create`] and [`requests::remove_window`]. `Remove { remove_worktree: true
/// }` does not: [`crate::manager::WindowManager::remove_with_worktree`] unregisters the
/// root itself, *before* deleting the checkout, and re-registers it if the deletion
/// fails, because that ordering (unregister, then delete, then maybe undo) has to happen
/// inside one operation or not be provable at all. **Do not add an `unregister` after
/// `remove_with_worktree` returns to "match" the other two paths** — the manager has
/// already unregistered by then, and a second unregister on top of its own is the exact
/// double-decrement `server::requests`' module doc and design decision 23 exist to
/// prevent.
pub async fn serve(
    listener: UnixListener,
    manager: Arc<WindowManager>,
    git: config::Git,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let (git_publish_tx, git_publish_rx) = mpsc::unbounded_channel();
    let git_registry = Arc::new(GitRegistry::new(git, git_publish_tx));
    let (git_tx, _) = broadcast::channel::<DaemonMsg>(256);
    tokio::spawn(pump_git(git_publish_rx, git_tx.clone(), shutdown.clone()));
    register_restored_roots(&manager, &git_registry);

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
                let git_registry = git_registry.clone();
                let git_tx = git_tx.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, manager, git_registry, git_tx, shutdown).await {
                        tracing::warn!(error = %e, "client connection ended with error");
                    }
                });
            }
        }
    }
}

/// Registers the worktree root of every window already in the table, before the first
/// client can connect.
///
/// At this point in `lifecycle::run` those are exactly the windows `WindowManager::restore`
/// loaded from `state.json`. Their checkouts are still on disk and still changing, so
/// git-surface spec 3.3's rule — "a root is registered when at least one window records
/// it" — applies to them no differently than to a created window; without this the
/// bottom bar is blank for every restored window, and stays blank, because `Restart`
/// does not register either (and must not: see below).
///
/// **Once per window, not once per distinct root.** Registration is reference counted
/// (`GitRegistry::register`), and every other site in this file pairs exactly one
/// `register` with one `unregister` per *window* — `requests::create` and
/// `requests::remove_window`. Deduplicating roots here would register one reference for
/// two restored windows on the same checkout, and the first `Remove` would then tear the
/// watcher down while the second window is still looking at it. Two restored windows on
/// one root have to behave exactly like two created ones, which means counting like
/// them.
///
/// **A restart registers nothing new**, for the same reason: a restarted window keeps
/// the record's root, which this call already holds a reference for. Adding a
/// `register` to the restart path would leak a reference per restart and leave the
/// watcher running after the window was removed.
///
/// Off the manager lock (AGENTS.md hard rule 10): `list()` returns owned `WindowInfo`s
/// and has released the lock before the first `register` runs.
fn register_restored_roots(manager: &WindowManager, git_registry: &GitRegistry) {
    for window in manager.list() {
        if let Some(root) = window.worktree {
            git_registry.register(root);
        }
    }
}

/// Turns every publication the registry makes into a `DaemonMsg::Git` broadcast.
///
/// A `broadcast::Sender::send` never blocks and never waits for a receiver, so nothing
/// here can be wedged by a slow or gone client — a lagging or dropped receiver only
/// affects that one client's own forwarding task in `handle_client`.
async fn pump_git(
    mut publish_rx: mpsc::UnboundedReceiver<(PathBuf, Option<GitState>)>,
    git_tx: broadcast::Sender<DaemonMsg>,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            received = publish_rx.recv() => match received {
                Some((root, state)) => {
                    // The registry already dedups (an unchanged poll never republishes),
                    // so every line here is a real change — this is what the milestone's
                    // manual check ("a `cargo build` must not storm the log") reads.
                    tracing::debug!(?root, ?state, "git state published");
                    let _ = git_tx.send(DaemonMsg::Git { root, state });
                }
                None => return,
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
    git_registry: Arc<GitRegistry>,
    git_tx: broadcast::Sender<DaemonMsg>,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let (mut rd, mut wr) = stream.into_split();

    // Decision 29: a client that connects and then says nothing (or stalls mid-frame)
    // must not hold this task, and the socket fd underneath it, forever.
    let hello = match tokio::time::timeout(
        proto::HANDSHAKE_TIMEOUT,
        read_frame::<_, ClientMsg>(&mut rd),
    )
    .await
    {
        Ok(frame) => frame?,
        Err(_) => {
            tracing::debug!("client did not send Hello within the handshake timeout; dropping");
            return Ok(());
        }
    };
    let Some(ClientMsg::Hello {
        proto_version,
        client,
    }) = hello
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
    // Subscribed before `Welcome` is even written, so nothing published between the
    // subscription and the snapshot replay below can be missed — at worst a root's
    // state is sent twice, never zero times.
    let mut git_rx = git_tx.subscribe();
    write_frame(
        &mut wr,
        &DaemonMsg::Welcome {
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            windows: manager.list(),
        },
    )
    .await?;
    // Design decision 17: every known root goes to a fresh client right after its
    // `Welcome`, written directly (nothing else touches `wr` yet), so a client's first
    // git traffic is never interleaved with anything else.
    for (root, state) in git_registry.snapshot() {
        write_frame(&mut wr, &DaemonMsg::Git { root, state }).await?;
    }

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

    // Forwards every later git publication to this client. A slow or gone client can
    // only stall this task's own `out_tx.send` (bounded by that client's channel), the
    // same way `changes_task` already can — it never touches `git_tx` itself, so it
    // cannot delay `pump_git` or any other client's forwarding task.
    let git_out = out_tx.clone();
    let git_task = tokio::spawn(async move {
        loop {
            match git_rx.recv().await {
                Ok(msg) => {
                    if git_out.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(missed = n, "client lagged on git broadcast");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Conversation subscriptions (task M6.5.10): answered and kept current by their own
    // task, so this loop never waits on one.
    let conversations =
        conversation::ConversationTask::spawn(manager.clone(), out_tx.clone(), shutdown.clone());

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
                // Runs in its own task, beside this loop, and is never aborted: the
                // request functions hand back no `JoinHandle` precisely so that nothing
                // here can cancel one (design decisions 17 and 25, `requests::detach`).
                requests::create(
                    manager.clone(),
                    git_registry.clone(),
                    out_tx.clone(),
                    spec,
                    cols,
                    rows,
                );
                None
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
            ClientMsg::Remove {
                window_id,
                remove_worktree,
                force,
            } => {
                if force && !remove_worktree {
                    // Design decision 20. `--force` is the user's answer to a dirty
                    // refusal and means nothing on its own, so it is refused rather than
                    // ignored: silently accepting it would let `anthrex rm --force` read
                    // as "remove harder" when the user forgot `--worktree`.
                    Some(error(
                        request::REMOVE,
                        "--force only applies when removing the worktree",
                    ))
                } else if remove_worktree {
                    // Like the create above, and for a sharper reason: this task
                    // unregisters the window's git root and then deletes its checkout,
                    // so a future dropped between those two steps would leave a live
                    // worktree that nothing watches — with the user's agent already
                    // killed and no restart in this milestone. Nothing in this function
                    // is given a handle with which to do that.
                    requests::remove_with_worktree(
                        manager.clone(),
                        git_registry.clone(),
                        out_tx.clone(),
                        window_id,
                        force,
                    );
                    None
                } else {
                    Some(requests::remove_window(&manager, &git_registry, window_id))
                }
            }
            ClientMsg::Rename { window_id, name } => {
                Some(ack_or_error("rename", manager.rename(window_id, name)))
            }
            ClientMsg::Restart { window_id } => {
                // Design decision 20: on its own spawned task, never awaited in this
                // loop. A live window's restart kills the old process and waits for it
                // to be gone (up to several seconds), and this loop must keep answering
                // every other message on the connection — in particular `ListWindows` —
                // for as long as that takes.
                requests::restart(manager.clone(), out_tx.clone(), window_id);
                None
            }
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
            ClientMsg::SubscribeConversation {
                window_id,
                agent_id,
                from_rev,
            } => {
                conversations.send(conversation::Command::Subscribe {
                    window_id,
                    agent_id,
                    from_rev,
                });
                None
            }
            ClientMsg::UnsubscribeConversation {
                window_id,
                agent_id,
            } => {
                conversations.send(conversation::Command::Unsubscribe {
                    window_id,
                    agent_id,
                });
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
    // Every conversation this client held is unsubscribed before the connection is
    // reported closed, as the PTY subscription above is unfocused.
    conversations.stop().await;
    changes_task.abort();
    git_task.abort();
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
                match reattach_with_snapshot(window_id, &out, &reattach).await {
                    Some(new_output) => output = new_output,
                    None => break,
                }
            }
            // Design decision 21: a restart swaps the window's `Process`, which drops
            // whatever this receiver was subscribed to — the old `Window`'s broadcast
            // sender, or a dormant window's capacity-1 sender that nothing was ever sent
            // on — and that is exactly what closes this channel. Ending the loop here, as
            // it used to, would silently stop forwarding a window's output the moment it
            // was restarted. `reattach()` under the hood takes the same lock as the swap
            // (`attach`), so it always sees whatever `Process` is current by the time it
            // runs: the new one if the swap already happened, or — if this task races
            // ahead of it — `reattach` simply fails and this loop ends, same as a window
            // that was removed outright.
            Err(broadcast::error::RecvError::Closed) => {
                match reattach_with_snapshot(window_id, &out, &reattach).await {
                    Some(new_output) => output = new_output,
                    None => break,
                }
            }
        }
    }
}

/// Shared by both `forward_output_from` recovery paths: re-attaches, sends a fresh
/// `Snapshot` built from what it got back, and hands the caller the new receiver to keep
/// reading from. `None` means either the re-attach itself failed (the window is gone) or
/// the snapshot could not be sent (the client is gone) — either way the caller's loop
/// ends.
async fn reattach_with_snapshot(
    window_id: u32,
    out: &mpsc::Sender<DaemonMsg>,
    reattach: &(impl Fn() -> anyhow::Result<Attachment> + Send),
) -> Option<broadcast::Receiver<Bytes>> {
    let att = reattach().ok()?;
    let snapshot = DaemonMsg::Snapshot {
        window_id,
        cols: att.cols,
        rows: att.rows,
        bytes: att.snapshot,
    };
    out.send(snapshot).await.ok()?;
    Some(att.output)
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
