//! Metering (M8b decisions 29 and 30).
//!
//! Every headless round is metered from its own stream (M8a decision 40); deciders and
//! scouts add their usage to the run (decision 29). The orchestrator, the one PTY run
//! session (milestone 9), is metered through Claude Code's OTLP export: [`server`]
//! receives it on loopback and [`otlp`] turns it into usage per `(run, role)`.
//!
//! - [`otlp`] is pure: parsing an OTLP/HTTP-JSON metrics body, the ledger, and the
//!   environment milestone 9 gives the orchestrator's window.
//! - [`server`] is the I/O: the listener, one task per connection, `<data_dir>/otlp.addr`.

pub mod otlp;
pub mod server;

pub use otlp::{OtlpLedger, UsageKind, UsagePoint, orchestrator_env, parse_metrics};
pub use server::{OtlpServer, bind, start};
