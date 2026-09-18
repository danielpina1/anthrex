//! The anthrex daemon: owns PTY windows and serves them over a Unix socket.

pub mod launch;
pub mod lifecycle;
pub mod manager;
pub mod server;
pub mod status;
pub mod window;

pub use lifecycle::{DaemonOptions, run};
