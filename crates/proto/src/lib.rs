//! Wire types shared by the daemon, the TUI client, and the CLI.

/// Bumped whenever a message shape changes incompatibly.
pub const PROTO_VERSION: u32 = 4;

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
    fn proto_version_is_four() {
        assert_eq!(super::PROTO_VERSION, 4);
    }
}
