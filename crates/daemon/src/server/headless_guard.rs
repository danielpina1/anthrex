//! Decision 49: a client can read a headless run session's window — list, rename, git
//! state, the conversation — but never drive it. The engine acts through the manager's
//! `headless_*` methods, which no client message reaches.
//!
//! `server.rs` asks here before it handles a message, so `Entry::attach`, the kill
//! escalation, removal and restart are never reached for a headless window. `Resize`
//! needs no refusal: the manager accepts and ignores it, with no reply, as it answers
//! any successful `Resize`.

use crate::manager::{
    WindowManager, control_refusal, lost_role_refusal, orchestrator_refusal, subscribe_refusal,
};
use proto::messages::request;
use proto::{ClientMsg, DaemonMsg};

/// The `DaemonMsg::Error` for `msg` when it is a `Subscribe`, `Input`, `Kill`, `Remove`
/// or `Restart` for a headless window, or a `Kill` or `Remove` for a live run's
/// orchestrator (milestone 9 decision 11); `None` for anything else, which the caller
/// then handles as before.
pub(super) fn refuse(manager: &WindowManager, msg: &ClientMsg) -> Option<DaemonMsg> {
    let (request, window_id) = match msg {
        ClientMsg::Subscribe { window_id, .. } => ("subscribe", *window_id),
        ClientMsg::Input { window_id, .. } => ("input", *window_id),
        ClientMsg::Kill { window_id } => ("kill", *window_id),
        ClientMsg::Remove { window_id, .. } => (request::REMOVE, *window_id),
        ClientMsg::Restart { window_id } => ("restart", *window_id),
        _ => return None,
    };
    if matches!(msg, ClientMsg::Kill { .. } | ClientMsg::Remove { .. }) {
        // Milestone 9 decision 11: a live run's orchestrator, a PTY window.
        if let Some(run) = manager.run_window_live(window_id) {
            let message = orchestrator_refusal(window_id, &run.run_id);
            let request = request.to_string();
            return Some(DaemonMsg::Error { request, message });
        }
        // Decision 11a: a placeholder headless window may be killed and removed.
        if manager.is_placeholder_headless(window_id) {
            return None;
        }
    }
    if matches!(msg, ClientMsg::Input { .. } | ClientMsg::Restart { .. })
        && let Some(run) = manager.lost_role_run(window_id)
    {
        let message = lost_role_refusal(window_id, &run);
        let request = request.to_string();
        return Some(DaemonMsg::Error { request, message });
    }
    let run = manager.headless_run(window_id)?;
    let message = match msg {
        ClientMsg::Subscribe { .. } => subscribe_refusal(window_id),
        _ => control_refusal(window_id, run.as_ref()),
    };
    Some(DaemonMsg::Error {
        request: request.to_string(),
        message,
    })
}
