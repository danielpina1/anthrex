//! Tails one transcript file (decisions A6 and A7). Blocking and synchronous: the daemon
//! drives [`Tail::read_more`] from `tokio::task::spawn_blocking`
//! (`crate::conversation::watch`), never from a tokio worker and never under the manager
//! lock (AGENTS.md hard rule 2). No child process, no tokio, no clock beyond one pass's
//! own deadline (decision A6).
//!
//! Every failure is a [`proto::DegradeReason`], never an `Err` (spec decision 2): a
//! transcript problem may cost the conversation its prose and tool detail, but never its
//! hook-built timeline.

use super::{Cursor, DETECT_LINES, Record, TranscriptParser, Version, detect_head};
use proto::DegradeReason;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// One pass over the file, on the blocking pool. Generous against a slow disk; a pass
/// that exceeds it returns what it has and the next poll continues from the same offset,
/// so overrunning costs latency, never correctness.
///
/// `read_more` checks it between chunks itself, so an ordinary slow pass stops on its
/// own; `crate::conversation::watch` also wraps the whole pass in a `tokio::time::timeout`
/// of this length, which only bites on a single `read` that blocks outright.
pub const TRANSCRIPT_READ_TIMEOUT: Duration = Duration::from_secs(2);
/// The most new bytes one pass consumes. An overnight run's transcript is tens of MiB;
/// this keeps any one pass bounded while the poll interval keeps the view current.
pub const TRANSCRIPT_READ_BUDGET: usize = 1024 * 1024;
/// A file larger than this is not read at all: `DegradeReason::TooLarge`.
pub const TRANSCRIPT_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// One line larger than this is skipped: `DegradeReason::BadRecord`. Sixteen times
/// `TOOL_RESULT_SUMMARY_MAX`'s 4 KiB is not the relationship here — a transcript line
/// can legitimately hold a whole file's contents — so this is bounded by memory, not by
/// a sibling constant.
pub const TRANSCRIPT_LINE_MAX: usize = 4 * 1024 * 1024;

/// How much one `read` call asks for. Only the granularity at which a pass checks its
/// deadline; the pass as a whole is bounded by `TRANSCRIPT_READ_BUDGET`.
const CHUNK: usize = 64 * 1024;

/// What the tail knows about the file's format.
#[derive(Debug)]
enum Format {
    /// `detect_head` has not answered yet; the complete lines seen so far, held (not
    /// parsed) until it does or until `DETECT_LINES` of them have passed.
    Detecting(Vec<String>),
    Known(Version),
    /// `DETECT_LINES` lines passed without an answer. Nothing more is parsed until the
    /// tail restarts.
    Unknown,
}

/// A position in one transcript file, carried from pass to pass.
#[derive(Debug)]
pub struct Tail {
    path: PathBuf,
    /// Bytes consumed so far: every complete line before it has been handed to the
    /// parser, and the bytes of an incomplete last line sit in `partial`.
    offset: u64,
    /// The bytes after the last `\n` read so far: a line still being written, never
    /// parsed until its `\n` arrives (decision A7). Bytes, not a `String`, because a
    /// pass can end in the middle of a multi-byte character.
    partial: Vec<u8>,
    /// Discarding the rest of a line that already exceeded `TRANSCRIPT_LINE_MAX`.
    skipping: bool,
    format: Format,
    /// Reset to `Cursor::default()` whenever the tail restarts (decision A7).
    cursor: Cursor,
    /// A line was broken or oversized since the last restart. Sticky: the records it
    /// cost do not come back on a later clean pass.
    bad_record: bool,
    /// `(dev, ino)` of the file last read, so a file replaced by a new one of the same
    /// or greater length is a restart too, not only one that shrank.
    identity: Option<(u64, u64)>,
}

/// One pass's result.
#[derive(Debug, Default, PartialEq)]
pub struct ReadOutcome {
    pub records: Vec<Record>,
    /// The file's state as of this pass. `Unreadable` and `TooLarge` describe this pass
    /// only; `UnknownFormat` and `BadRecord` hold until the tail restarts.
    pub degraded: Option<DegradeReason>,
    /// True when the file shrank or was replaced and the tail restarted (decision A7):
    /// the caller discards its enrichment before applying `records`.
    pub restarted: bool,
}

impl Tail {
    pub fn new(path: PathBuf) -> Self {
        Tail {
            path,
            offset: 0,
            partial: Vec::new(),
            skipping: false,
            format: Format::Detecting(Vec::new()),
            cursor: Cursor::default(),
            bad_record: false,
            identity: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bytes of the file consumed so far.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Blocking. Never returns an `Err`: every failure is a `DegradeReason`.
    pub fn read_more(&mut self, parser: &dyn TranscriptParser) -> ReadOutcome {
        self.read_more_until(parser, Instant::now() + TRANSCRIPT_READ_TIMEOUT)
    }

    fn read_more_until(&mut self, parser: &dyn TranscriptParser, deadline: Instant) -> ReadOutcome {
        let mut outcome = ReadOutcome::default();
        // Non-blocking, so a FIFO planted at the path cannot hang the blocking thread in
        // `open`; for a regular file the flag changes nothing.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&self.path);
        let Ok(mut file) = file else {
            return self.finish(outcome, Some(DegradeReason::Unreadable));
        };
        let Ok(meta) = file.metadata() else {
            return self.finish(outcome, Some(DegradeReason::Unreadable));
        };
        if !meta.file_type().is_file() {
            return self.finish(outcome, Some(DegradeReason::Unreadable));
        }
        if meta.len() > TRANSCRIPT_MAX_BYTES {
            return self.finish(outcome, Some(DegradeReason::TooLarge));
        }
        let identity = (meta.dev(), meta.ino());
        if meta.len() < self.offset || self.identity.is_some_and(|known| known != identity) {
            self.restart();
            outcome.restarted = true;
        }
        self.identity = Some(identity);
        if matches!(self.format, Format::Unknown) {
            return self.finish(outcome, None);
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return self.finish(outcome, Some(DegradeReason::Unreadable));
        }

        let mut buf = vec![0u8; CHUNK];
        let mut consumed = 0usize;
        let mut failed = false;
        while consumed < TRANSCRIPT_READ_BUDGET && !matches!(self.format, Format::Unknown) {
            let want = CHUNK.min(TRANSCRIPT_READ_BUDGET - consumed);
            let n = match file.read(&mut buf[..want]) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    failed = true;
                    break;
                }
            };
            consumed += n;
            self.offset += n as u64;
            self.feed(parser, &buf[..n], &mut outcome.records);
            if Instant::now() >= deadline {
                break;
            }
        }
        self.finish(outcome, failed.then_some(DegradeReason::Unreadable))
    }

    /// Fills in `degraded`: this pass's own failure if it had one, else the tail's
    /// standing state.
    fn finish(&self, mut outcome: ReadOutcome, failure: Option<DegradeReason>) -> ReadOutcome {
        outcome.degraded = failure.or(match self.format {
            Format::Unknown => Some(DegradeReason::UnknownFormat),
            _ => self.bad_record.then_some(DegradeReason::BadRecord),
        });
        outcome
    }

    /// Decision A7: the offset, the partial line, the version and the cursor go back to
    /// the start together.
    fn restart(&mut self) {
        *self = Tail::new(std::mem::take(&mut self.path));
    }

    /// Splits `bytes` into lines, carrying an incomplete last line in `partial`.
    fn feed(&mut self, parser: &dyn TranscriptParser, mut bytes: &[u8], out: &mut Vec<Record>) {
        while !bytes.is_empty() {
            let newline = bytes.iter().position(|&b| b == b'\n');
            let (segment, rest) = match newline {
                Some(at) => (&bytes[..at], &bytes[at + 1..]),
                None => (bytes, &[][..]),
            };
            bytes = rest;
            if self.skipping {
                if newline.is_some() {
                    self.skipping = false;
                }
                continue;
            }
            self.partial.extend_from_slice(segment);
            if self.partial.len() > TRANSCRIPT_LINE_MAX {
                self.partial = Vec::new();
                self.bad_record = true;
                self.skipping = newline.is_none();
                continue;
            }
            if newline.is_some() {
                let line = std::mem::take(&mut self.partial);
                self.line(parser, &line, out);
                if matches!(self.format, Format::Unknown) {
                    return;
                }
            }
        }
    }

    fn line(&mut self, parser: &dyn TranscriptParser, bytes: &[u8], out: &mut Vec<Record>) {
        let Ok(line) = std::str::from_utf8(bytes) else {
            self.bad_record = true;
            return;
        };
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        match &mut self.format {
            Format::Known(version) => {
                let version = *version;
                self.parse(parser, version, line, out);
            }
            Format::Detecting(head) => {
                head.push(line.to_owned());
                match detect_head(parser, head.iter().map(String::as_str)) {
                    Some(version) => {
                        let head = std::mem::take(head);
                        self.format = Format::Known(version);
                        for held in &head {
                            self.parse(parser, version, held, out);
                        }
                    }
                    None if head.len() >= DETECT_LINES => self.format = Format::Unknown,
                    None => {}
                }
            }
            Format::Unknown => {}
        }
    }

    fn parse(
        &mut self,
        parser: &dyn TranscriptParser,
        version: Version,
        line: &str,
        out: &mut Vec<Record>,
    ) {
        if parser.malformed(version, line) {
            self.bad_record = true;
            return;
        }
        out.extend(parser.record(version, line, &mut self.cursor));
    }
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
