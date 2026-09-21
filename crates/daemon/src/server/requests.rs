//! The client requests that can run git, and the rule they share: they are never
//! abandoned.
//!
//! `CreateWindow` with a branch and `Remove` with `remove_worktree` both spawn a child
//! process, take a deadline measured in tens of seconds, and change what is on disk. Two
//! things follow, and they are the whole reason these three functions are not written
//! inline in [`super::handle_client`]'s match.
//!
//! # They run beside the connection loop, not inside it
//!
//! Design decision 25. A request awaited in the loop would stall every *other* message on
//! that connection for as long as git took — input to other windows, resizes, the periodic
//! `ListWindows` a client uses to notice anything — so each one is handed to its own task
//! and the loop goes straight on to the next frame. `Subscribe` deliberately stays in the
//! loop: milestone 1's rule that a `Snapshot` is queued before any live output for the same
//! window is an ordering between two sends on one channel, and only the loop can keep it.
//!
//! # They are never aborted
//!
//! Design decisions 17 and 25, and the sharper of the two constraints. A create's phase B
//! runs to completion on the blocking pool whatever its caller does, so a create whose
//! future is dropped leaves a checkout and a branch on disk with no window attached to
//! them and nothing that will ever clean them up. A removal is worse:
//! [`crate::manager::WindowManager::remove_with_worktree`] kills the agent, unregisters the
//! window's git root, deletes the checkout, and re-registers the root if the deletion
//! fails. A future dropped between the unregister and the removal skips both the
//! re-`register` and the deletion, which leaves a live worktree that nothing watches — and
//! by then the user's agent is already dead, with no restart in this milestone to get it
//! back.
//!
//! [`detach`] is where that guarantee is made, in one place, on purpose: see its own note.

use super::{ack_or_error, error};
use crate::git::GitRegistry;
use crate::manager::{RemoveError, WindowManager};
use proto::messages::request;
use proto::{DaemonMsg, WindowSpec};
use std::future::Future;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Runs one long request to completion, beside the connection loop and outliving it.
///
/// The dropped `JoinHandle` is the point of this function. `tokio::spawn` runs a future on
/// the runtime whether or not anyone keeps its handle, and dropping the handle *detaches*
/// the task rather than cancelling it — so a task spawned here finishes even after the
/// client that asked for it has gone and [`super::handle_client`] has returned. That is
/// exactly what design decisions 17 and 25 require, and it is fragile in a way a bare
/// `tokio::spawn(...)` statement hides: `handle_client` already keeps and aborts two
/// handles of its own (`changes_task` and `git_task`), so binding one more to a local and
/// aborting it alongside them would read like housekeeping and would silently start
/// tearing worktree operations in half.
///
/// Naming the drop gives that invariant somewhere to live and somewhere to be found.
/// `crates/daemon/tests/server/worktree.rs` pins it from the outside: both
/// `..._after_its_client_disconnects` tests take the connection away at a moment they have
/// proved is mid-operation, and assert the work finished anyway.
fn detach(request: impl Future<Output = ()> + Send + 'static) {
    drop(tokio::spawn(request));
}

/// `CreateWindow`: resolve the roots, create the window, register whatever root it ended
/// up in.
///
/// The `register` is after `create` has returned and its lock has been released — never
/// from inside the blocking closure, and never before the window exists (AGENTS.md hard
/// rule 10, design decision 16). `create` does its own `spawn_blocking` for every step
/// that can stall, so there is no second blocking hop here: one would only put the
/// lock-held phases back on a blocking thread for nothing.
pub(super) fn create(
    manager: Arc<WindowManager>,
    git_registry: Arc<GitRegistry>,
    out: mpsc::Sender<DaemonMsg>,
    spec: WindowSpec,
    cols: u16,
    rows: u16,
) {
    detach(async move {
        let roots = crate::project::resolve_roots(spec.cwd.clone()).await;
        if let Some(message) = detection_failed_for_a_worktree_request(&spec, &roots) {
            reply_to(&out, error(request::CREATE, message)).await;
            return;
        }
        let result = manager
            .create(spec, roots.project, roots.worktree, cols, rows)
            .await;
        let reply = match result {
            Ok(info) => {
                // For a worktree window this is the new linked checkout, not the directory
                // the user pointed at: `create`'s phase C replaced it (design decision 21).
                if let Some(root) = info.worktree.clone() {
                    git_registry.register(root);
                }
                DaemonMsg::Created { window_id: info.id }
            }
            Err(e) => error(request::CREATE, e.to_string()),
        };
        reply_to(&out, reply).await;
    });
}

/// Fix wave C item 5: `worktree::create` refuses a worktree branch request with
/// `WorktreeError::NotARepo` whenever `roots.worktree` is `None`, worded as a flat claim
/// that the directory is not a git repository. That claim is right when detection
/// genuinely found nothing, but wrong — and actively misleading, sending a user hunting
/// for a bug in a checkout that is perfectly fine — when detection simply could not
/// finish (`DetectedRoots::detection_failed`'s doc comment lists why).
///
/// Checked here, before `manager.create` is ever called, because this is the one place
/// that distinction still exists: `WindowManager::create` takes `worktree: Option<PathBuf>`
/// rather than a whole `DetectedRoots`, on purpose (`create.rs`'s comment on its own
/// reconstruction of one), so by the time `worktree::create` sees `worktree: None` there
/// is no way left to tell "detection failed" from "detection answered no". Intercepting
/// here means neither the manager nor `worktree::create` has to carry a field that exists
/// for exactly one caller.
fn detection_failed_for_a_worktree_request(
    spec: &WindowSpec,
    roots: &crate::project::DetectedRoots,
) -> Option<String> {
    if spec.worktree_branch.is_some() && roots.worktree.is_none() && roots.detection_failed {
        Some(format!(
            "could not tell whether {} is a git repository: root detection failed; try again",
            spec.cwd.display()
        ))
    } else {
        None
    }
}

/// `Remove { remove_worktree: true }`: the window and the checkout this daemon made for
/// it.
///
/// Nothing here touches the git registry. The unregister has to happen between the kill
/// and the deletion, and to be undone if the deletion fails, which is an ordering only
/// something *inside* the operation can produce — so the manager owns it and is handed the
/// registry as a [`crate::manager::GitRoots`] (design decision 22). A tidy-looking
/// `git_registry.unregister(...)` added here after the call would be a second
/// decrement of one registration.
///
/// [`RemoveError::Dirty`] is the one failure that is a question rather than an answer, so
/// it goes back as its own message and the client turns it into the force-or-keep prompt
/// (design decision 24). Everything else is an ordinary [`request::REMOVE`] error.
///
/// `window_id` rides back on that message rather than being left for the client to infer
/// from whatever removal it thinks is outstanding: the prompt it opens offers a
/// `--force` deletion, and the client must be able to prove the id it forces is the id
/// this refusal is about. See [`DaemonMsg::RemoveDirty`].
pub(super) fn remove_with_worktree(
    manager: Arc<WindowManager>,
    git_registry: Arc<GitRegistry>,
    out: mpsc::Sender<DaemonMsg>,
    window_id: u32,
    force: bool,
) {
    detach(async move {
        let reply = match manager
            .remove_with_worktree(window_id, force, &*git_registry)
            .await
        {
            Ok(()) => DaemonMsg::Ack {
                request: request::REMOVE.to_string(),
            },
            Err(RemoveError::Dirty(dirty)) => DaemonMsg::RemoveDirty {
                window_id,
                message: dirty.to_string(),
            },
            Err(RemoveError::Failed(e)) => error(request::REMOVE, e.to_string()),
        };
        reply_to(&out, reply).await;
    });
}

/// `Remove { remove_worktree: false }`: the window only, with any worktree left on disk
/// (design decision 18).
///
/// Synchronous and answered from the loop, because it is: forgetting an entry is a
/// `BTreeMap` removal and a signal.
///
/// The root is captured before `remove`, never held across it: `list` and `remove` each
/// take and release the manager lock on their own, so nothing here runs with it held
/// (AGENTS.md hard rule 2).
///
/// Whether this was the *last* reference to the root is [`GitRegistry`]'s own call, not
/// this handler's (design decision 23): deriving it here from a second `manager.list()`
/// would race a concurrent `CreateWindow` on the same root across two independent locks —
/// the manager's and the registry's — with nothing to order them, so a `register` and this
/// `unregister` could land in either order and leave the root permanently unregistered
/// while a window still used it. `unregister` is called unconditionally instead, and the
/// registry's own reference count decides whether anything actually stops.
///
/// Unconditionally, but only on success, and that is not a detail. `remove` refuses a
/// window that [`remove_with_worktree`] has already admitted, so a plain removal racing a
/// worktree removal of the same window returns an error here and unregisters nothing;
/// without that refusal both paths would decrement one registration, and with two windows
/// on a root — a restart or an attach — the survivor's watch would stop with nothing on
/// screen to explain why.
pub(super) fn remove_window(
    manager: &WindowManager,
    git_registry: &GitRegistry,
    window_id: u32,
) -> DaemonMsg {
    let removed_root = manager
        .list()
        .into_iter()
        .find(|w| w.id == window_id)
        .and_then(|w| w.worktree);
    let result = manager.remove(window_id);
    if result.is_ok()
        && let Some(root) = removed_root
    {
        git_registry.unregister(&root);
    }
    ack_or_error(request::REMOVE, result)
}

/// Sends a detached request's reply, if the client is still there to receive it.
///
/// Best effort by design (decision 25): the request has already happened, and a client
/// that disconnected mid-operation cannot be told about it. Silence here is normal, not a
/// failure of the request.
async fn reply_to(out: &mpsc::Sender<DaemonMsg>, reply: DaemonMsg) {
    if out.send(reply).await.is_err() {
        tracing::debug!("client gone before its reply; the request itself ran to completion");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::DetectedRoots;
    use proto::Runtime;
    use std::path::PathBuf;

    fn spec(worktree_branch: Option<&str>) -> WindowSpec {
        WindowSpec {
            name: None,
            runtime: Runtime::Shell,
            cwd: PathBuf::from("/work"),
            worktree_branch: worktree_branch.map(str::to_string),
            model: None,
            initial_prompt: None,
        }
    }

    fn roots(worktree: Option<&str>, detection_failed: bool) -> DetectedRoots {
        DetectedRoots {
            project: PathBuf::from("/work"),
            worktree: worktree.map(PathBuf::from),
            detection_failed,
        }
    }

    /// The one case this function exists to catch: a worktree branch was asked for,
    /// detection found no repository, and it could not even tell whether there was one.
    #[test]
    fn a_worktree_request_whose_detection_failed_gets_a_message_that_says_so() {
        let message =
            detection_failed_for_a_worktree_request(&spec(Some("feat/x")), &roots(None, true))
                .expect("a failed detection behind a worktree request must not be silent");
        assert!(message.contains("/work"), "{message}");
        assert!(
            message.contains("detection failed"),
            "the message must say detection itself is what failed: {message}"
        );
        assert!(
            !message.contains("not a git repository"),
            "that claim is exactly the one this case must not make: {message}"
        );
        assert!(
            message.contains("try again"),
            "a transient failure should say so is worth retrying: {message}"
        );
    }

    /// A worktree branch was asked for and detection genuinely found no repository —
    /// `worktree::create`'s `WorktreeError::NotARepo` is right here, so this function
    /// must stay out of the way and let that path run.
    #[test]
    fn a_worktree_request_with_a_real_negative_answer_is_not_intercepted() {
        assert_eq!(
            detection_failed_for_a_worktree_request(&spec(Some("feat/x")), &roots(None, false)),
            None,
            "detection answered 'no repository here'; that is worktree::create's message \
             to give, not this function's"
        );
    }

    /// A plain window (no worktree requested) never reaches `worktree::create` at all, so
    /// a failed detection is nothing to report here — `roots.project`'s fallback is all a
    /// plain window ever needed.
    #[test]
    fn a_plain_window_is_never_intercepted_even_if_detection_failed() {
        assert_eq!(
            detection_failed_for_a_worktree_request(&spec(None), &roots(None, true)),
            None
        );
    }

    /// Detection found a real worktree; a stale or irrelevant `detection_failed` on a
    /// `Some` answer must not matter.
    #[test]
    fn a_worktree_request_that_found_a_worktree_is_never_intercepted() {
        assert_eq!(
            detection_failed_for_a_worktree_request(
                &spec(Some("feat/x")),
                &roots(Some("/work"), true)
            ),
            None
        );
    }
}
