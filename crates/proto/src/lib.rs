//! Wire types shared by the daemon, the TUI client, and the CLI.

/// Bumped whenever a message shape changes incompatibly — or whenever an existing
/// field's *meaning* changes, which is why this is 5 and not 4.
///
/// Milestone 5 adds one message — `DaemonMsg::RemoveDirty`, whose own doc comment says
/// why the dirty refusal has to carry a window id — and changes what
/// `ClientMsg::Remove { remove_worktree }` means: a milestone-4.5 daemon ignores that
/// field entirely, because no window it can create has a worktree, and answers
/// `Ack { request: "remove" }` either way. A milestone-5 client paired with one would ask
/// for a worktree to be deleted and be told the removal succeeded while nothing was
/// removed. A silent wrong answer is precisely what a version number exists to prevent, so
/// the number moves and the handshake refuses the pairing instead.
pub const PROTO_VERSION: u32 = 5;

/// How long the daemon waits for a freshly connected client's `Hello`, and how long a
/// client waits for the daemon's `Welcome`, before giving up on the handshake. Design
/// decision 29: a client that connects and then says nothing must not hold a daemon
/// resource forever, and a daemon that accepted but never answers must not hang a client
/// indefinitely either.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub mod codec;
pub mod messages;
pub mod paths;
pub mod types;

pub use codec::{CodecError, MAX_FRAME, decode, encode, read_frame, write_frame};
pub use messages::{ClientMsg, DaemonMsg, HookSource};
pub use types::{
    ClientKind, ExitInfo, GitOperation, GitState, Head, Runtime, Status, SubagentInfo,
    SubagentState, WindowInfo, WindowSpec,
};

#[cfg(test)]
mod tests {
    #[test]
    fn proto_version_is_five() {
        assert_eq!(super::PROTO_VERSION, 5);
    }

    #[test]
    fn handshake_timeout_is_five_seconds() {
        assert_eq!(super::HANDSHAKE_TIMEOUT, std::time::Duration::from_secs(5));
    }
}
