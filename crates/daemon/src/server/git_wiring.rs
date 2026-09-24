//! The daemon's one [`GitRegistry`] and what feeds its publications to clients (design
//! decision 22 of milestone 8a: the registry is built once and shared by the server and
//! the run engine).
//!
//! One registry watches and probes whatever roots the daemon's windows reference, and one
//! broadcast channel turns its publications into [`DaemonMsg::Git`] for every attached
//! client. The manager itself holds no git state and takes no git-related lock (AGENTS.md
//! hard rule 2).

use crate::git::GitRegistry;
use crate::manager::WindowManager;
use proto::{DaemonMsg, GitState};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

/// The registry and the receiving end of its publications, which `server::serve` pumps
/// into its client broadcast.
pub struct GitWiring {
    pub registry: Arc<GitRegistry>,
    pub publish_rx: mpsc::UnboundedReceiver<(PathBuf, Option<GitState>)>,
}

impl GitWiring {
    pub fn new(settings: config::Git) -> Self {
        let (publish_tx, publish_rx) = mpsc::unbounded_channel();
        GitWiring {
            registry: Arc::new(GitRegistry::new(settings, publish_tx)),
            publish_rx,
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
/// (`GitRegistry::register`), and every other site in `server` pairs exactly one
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
pub(super) fn register_restored_roots(manager: &WindowManager, git_registry: &GitRegistry) {
    // A headless window's root is the run engine's to watch (decision 22): it
    // registers each run worktree once, whatever windows it has.
    for window in manager.list() {
        if window.kind == proto::WindowKind::Headless {
            continue;
        }
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
pub(super) async fn pump_git(
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
