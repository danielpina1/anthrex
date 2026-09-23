//! Wire types shared by the daemon, the TUI client, and the CLI.

/// Bumped whenever a message shape changes incompatibly — or whenever an existing
/// field's *meaning* changes, which is why this is 6 and not 5.
///
/// Milestone 6.5 adds the agent conversation model (`proto::conversation`) and five
/// messages built on it: `ClientMsg::SubscribeConversation` and
/// `ClientMsg::UnsubscribeConversation`, and `DaemonMsg::ConversationSnapshot`,
/// `DaemonMsg::ConversationDelta` and `DaemonMsg::ConversationGone`. None of these replace
/// or change the meaning of an existing field, but a milestone-5 daemon paired with a
/// milestone-6.5 client (or the reverse) has no way to decode a message it has never seen
/// — `read_frame` would hand back a decode error indistinguishable from a corrupt frame,
/// on a connection that otherwise looks healthy. The version bump turns that into a clean
/// handshake refusal instead, which is what every earlier bump on this line has done for
/// a genuinely new shape, not only for a changed one.
///
/// Task M6.5.10 added `session_id` to `ConversationDelta` without a further bump: version
/// 6 has not shipped, so no client or daemon speaking a 6 without it exists.
pub const PROTO_VERSION: u32 = 6;

/// How long the daemon waits for a freshly connected client's `Hello`, and how long a
/// client waits for the daemon's `Welcome`, before giving up on the handshake. Design
/// decision 29: a client that connects and then says nothing must not hold a daemon
/// resource forever, and a daemon that accepted but never answers must not hang a client
/// indefinitely either.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub mod codec;
pub mod conversation;
pub mod messages;
pub mod paths;
pub mod types;

pub use codec::{CodecError, MAX_FRAME, decode, encode, read_frame, write_frame};
pub use conversation::{
    Block, Conversation, DegradeReason, DropCause, NoticeKind, Role, ToolResult, ToolState, Turn,
    TurnPatch, TurnState,
};
pub use messages::{ClientMsg, DaemonMsg, HookSource};
pub use types::{
    ClientKind, ExitInfo, GitOperation, GitState, Head, Runtime, Status, SubagentInfo,
    SubagentState, WindowInfo, WindowSpec,
};

#[cfg(test)]
mod tests {
    #[test]
    fn proto_version_is_six() {
        assert_eq!(super::PROTO_VERSION, 6);
    }

    #[test]
    fn handshake_timeout_is_five_seconds() {
        assert_eq!(super::HANDSHAKE_TIMEOUT, std::time::Duration::from_secs(5));
    }
}
