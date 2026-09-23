//! The transcript reader task (decision 9, task M6.5.10): one per window, alive while at
//! least one client is subscribed to one of that window's conversations, plus
//! `conversation.linger_secs`.
//!
//! Where each piece runs (AGENTS.md hard rule 2):
//! - [`WindowManager::reader_step`] and [`WindowManager::apply_transcript`] take the
//!   manager lock for in-memory work only: deciding, moving the `Tail` out and back,
//!   applying records.
//! - [`Tail::read_more`](crate::transcript::reader::Tail::read_more), the only code that
//!   touches the file, runs on `tokio::task::spawn_blocking`, with no lock held.
//! - This loop itself only awaits.

use crate::manager::{ReaderStep, WindowManager};
use crate::transcript::TranscriptParser;
use crate::transcript::reader::TRANSCRIPT_READ_TIMEOUT;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// How often a subscribed conversation's transcript is re-read.
pub const TRANSCRIPT_POLL: Duration = Duration::from_millis(250);

/// Spawns the reader for `window_id`. The caller has already marked the window's reader
/// as running, under the manager lock, so there is never a second one.
///
/// Each tick reads first and then sleeps, so a new subscriber's first answer is followed
/// by the transcript's contents one pass later rather than one poll later.
///
/// A pass that outlives `TRANSCRIPT_READ_TIMEOUT` (`transcript/reader.rs`) — which
/// `read_more` avoids on its own by checking the deadline between chunks, so this means a
/// single `read` blocked outright — is waited for again rather than abandoned: the pass
/// owns the `Tail`, and starting a second pass without it would read the file twice. The
/// overrun is logged each time it recurs; it costs this window's view latency, and
/// nothing else, because no lock is held.
pub fn spawn_reader(
    manager: Arc<WindowManager>,
    window_id: u32,
    parser: &'static dyn TranscriptParser,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(window_id, "transcript reader started");
        loop {
            match manager.reader_step(window_id) {
                ReaderStep::Stop => break,
                ReaderStep::Wait => {}
                ReaderStep::Read(mut tail) => {
                    let mut pass = tokio::task::spawn_blocking(move || {
                        let outcome = tail.read_more(parser);
                        (tail, outcome)
                    });
                    let finished = loop {
                        tokio::select! {
                            _ = shutdown.cancelled() => return,
                            waited = tokio::time::timeout(TRANSCRIPT_READ_TIMEOUT, &mut pass) => {
                                match waited {
                                    Ok(finished) => break finished,
                                    Err(_) => tracing::warn!(
                                        window_id,
                                        "transcript read pass overran; still waiting for it"
                                    ),
                                }
                            }
                        }
                    };
                    match finished {
                        Ok((tail, outcome)) => manager.apply_transcript(window_id, tail, outcome),
                        Err(error) => {
                            tracing::error!(window_id, %error, "transcript read pass failed");
                            manager.transcript_lost(window_id);
                        }
                    }
                }
            }
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(TRANSCRIPT_POLL) => {}
            }
        }
        tracing::info!(window_id, "transcript reader stopped");
    })
}
