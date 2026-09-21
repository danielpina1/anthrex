//! Wire types shared by the daemon, the TUI client, and the CLI.

/// Bumped whenever a message shape changes incompatibly — or whenever an existing
/// field's *meaning* changes, which is why this is 5 and not 4.
///
/// Milestone 5 adds no message and changes no shape. It changes what
/// `ClientMsg::Remove { remove_worktree }` means: a milestone-4.5 daemon ignores that
/// field entirely, because no window it can create has a worktree, and answers
/// `Ack { request: "remove" }` either way. A milestone-5 client paired with one would ask
/// for a worktree to be deleted and be told the removal succeeded while nothing was
/// removed. A silent wrong answer is precisely what a version number exists to prevent, so
/// the number moves and the handshake refuses the pairing instead.
pub const PROTO_VERSION: u32 = 5;

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
}
