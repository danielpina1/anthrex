//! Wire types shared by the daemon, the TUI client, and the CLI.

/// Bumped whenever a message shape changes incompatibly.
pub const PROTO_VERSION: u32 = 1;

pub mod messages;
pub mod types;

pub use messages::{ClientMsg, DaemonMsg, HookSource};
pub use types::{ClientKind, ExitInfo, Runtime, Status, WindowInfo, WindowSpec};
