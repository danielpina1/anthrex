//! [`run_captured_head_tail`]: [`super::run_captured`]'s hardening for an output of
//! any size, keeping only its two ends. Split from `subprocess.rs` for AGENTS.md rule 8.

use std::io;
use std::process::Command;
use std::time::Duration;

use super::{End, Outcome, StdoutSink, capture};

/// The first bytes of a child's stdout, its last bytes, and how many it wrote in all:
/// what [`run_captured_head_tail`] keeps of an output that may be far too large to
/// hold. `tail` holds the bytes after `head` only; when `total` exceeds
/// `head.len() + tail.len()`, the bytes between them were read and dropped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeadTail {
    pub head: Vec<u8>,
    pub tail: Vec<u8>,
    pub total: u64,
}

impl HeadTail {
    /// Whether bytes between `head` and `tail` were dropped.
    pub fn dropped(&self) -> bool {
        self.total > (self.head.len() + self.tail.len()) as u64
    }
}

/// As [`run_captured`], but stdout is never over a cap: the first `head_bytes` are
/// kept, then a ring of the last `tail_bytes`, and everything between is read and
/// dropped, so an arbitrarily large output (a diff of a vendored data file) neither
/// fails the command nor is held in memory. `Complete`, `TimedOut` and `Failed` mean
/// what they mean for [`run_captured`]; there is no `Truncated`.
pub fn run_captured_head_tail(
    command: &mut Command,
    head_bytes: usize,
    tail_bytes: usize,
    max_stderr_bytes: usize,
    timeout: Duration,
) -> (Outcome, HeadTail, String, Option<io::ErrorKind>) {
    let mut sink = HeadTailSink {
        head_max: head_bytes,
        tail_max: tail_bytes,
        kept: HeadTail::default(),
        ring: std::collections::VecDeque::new(),
    };
    let (end, stderr, spawn_error) = capture(command, &mut sink, max_stderr_bytes, timeout);
    let mut kept = sink.kept;
    kept.tail = sink.ring.into_iter().collect();
    let outcome = match end {
        End::Failed | End::OverCap => Outcome::Failed,
        End::Complete => Outcome::Complete(Vec::new()),
        End::TimedOut => Outcome::TimedOut(Vec::new()),
    };
    (outcome, kept, stderr, spawn_error)
}

struct HeadTailSink {
    head_max: usize,
    tail_max: usize,
    kept: HeadTail,
    ring: std::collections::VecDeque<u8>,
}

impl StdoutSink for HeadTailSink {
    fn accept(&mut self, mut bytes: &[u8]) -> bool {
        self.kept.total += bytes.len() as u64;
        let room = self.head_max.saturating_sub(self.kept.head.len());
        let into_head = room.min(bytes.len());
        self.kept.head.extend_from_slice(&bytes[..into_head]);
        bytes = &bytes[into_head..];
        if bytes.len() >= self.tail_max {
            self.ring.clear();
            self.ring.extend(&bytes[bytes.len() - self.tail_max..]);
        } else {
            self.ring.extend(bytes);
            let excess = self.ring.len().saturating_sub(self.tail_max);
            self.ring.drain(..excess);
        }
        true
    }
}
