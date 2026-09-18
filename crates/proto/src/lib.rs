//! Wire types shared by the daemon, the TUI client, and the CLI.

/// Bumped whenever a message shape changes incompatibly.
pub const PROTO_VERSION: u32 = 2;

pub mod codec;
pub mod messages;
pub mod paths;
pub mod types;

pub use codec::{CodecError, MAX_FRAME, decode, encode, read_frame, write_frame};
pub use messages::{ClientMsg, DaemonMsg, HookSource};
pub use types::{
    ClientKind, ExitInfo, Runtime, Status, SubagentInfo, SubagentState, WindowInfo, WindowSpec,
};
