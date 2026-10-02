//! [`run_captured_keeping`]: [`super::run_captured`], with stdout returned whatever the
//! exit status. Milestone 9.2 (task M9.2.4): `git push --porcelain` reports a refused
//! ref on stdout and exits 1, and only that line tells a non-fast-forward (`[rejected]`)
//! from a remote's refusal (`[remote rejected]`, ruling R-11). Split from
//! `subprocess.rs` for AGENTS.md rule 8.

use std::process::Command;
use std::time::Duration;

use super::{CappedSink, Captured, End, Outcome, capture};

/// As [`super::run_captured`], and the stdout read is also returned when the child exits
/// non-zero (`Outcome::Failed`) or times out; `Outcome::Complete`'s own vector is then
/// empty, the output being the second element. Over its cap, the output is what was read
/// before the cap.
pub fn run_captured_keeping(
    command: &mut Command,
    max_output_bytes: usize,
    max_stderr_bytes: usize,
    timeout: Duration,
) -> (Captured, Vec<u8>) {
    let mut sink = CappedSink {
        output: Vec::new(),
        max: max_output_bytes,
    };
    let (end, stderr, spawn_error) = capture(command, &mut sink, max_stderr_bytes, timeout);
    let outcome = match end {
        End::Failed => Outcome::Failed,
        End::Complete => Outcome::Complete(Vec::new()),
        End::TimedOut => Outcome::TimedOut(Vec::new()),
        End::OverCap => Outcome::Truncated(Vec::new()),
    };
    (
        Captured {
            outcome,
            stderr,
            spawn_error,
        },
        sink.output,
    )
}
