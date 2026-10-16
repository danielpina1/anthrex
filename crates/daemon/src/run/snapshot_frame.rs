//! The snapshot's transport frame (milestone 9.11 decision 10): what keeps a pushed
//! snapshot within one frame. Moved out of `snapshot.rs` (milestone 9.17, ruling C1).
//! Pure: it measures and trims a snapshot, no I/O beyond the error log.

use proto::RunsSnapshot;

/// Decision 10: room left in a frame for what wraps the snapshot.
pub const FRAME_MARGIN: usize = 64 * 1024;

/// [`fit_within`] one frame, less [`FRAME_MARGIN`]: what `publish` and `current` send.
pub fn fit_frame(snap: RunsSnapshot) -> RunsSnapshot {
    fit_within(snap, proto::MAX_FRAME - FRAME_MARGIN)
}

/// Decision 10's last guard: while `snap` encodes over `max` bytes, its oldest finished
/// run is dropped (the list is newest first); a live run never is. When only live runs
/// are left and it is still too large, it is returned as it is and the largest run is
/// logged (the connection writer then drops the frame, `server.rs`).
pub(crate) fn fit_within(mut snap: RunsSnapshot, max: usize) -> RunsSnapshot {
    loop {
        let size = encoded_len(&snap);
        if size <= max {
            return snap;
        }
        // Each finished run's own encoded size counts against the excess, so one pass
        // drops enough; the loop re-measures, as the list's own header can shrink too.
        let mut excess = size - max;
        let mut dropped = false;
        while excess > 0 {
            let Some(i) = snap.runs.iter().rposition(|r| r.state.is_terminal()) else {
                break;
            };
            excess = excess.saturating_sub(encoded_len(&snap.runs.remove(i)));
            dropped = true;
        }
        if !dropped {
            let largest = (snap.runs.iter())
                .map(|r| (encoded_len(r), r.run_id.as_str()))
                .max();
            let (run_bytes, run) = largest.unwrap_or((0, ""));
            tracing::error!(
                size,
                max,
                run,
                run_bytes,
                "the run snapshot is over one frame with only live runs left; sent as it is"
            );
            return snap;
        }
    }
}

/// `value`'s MessagePack body size, as a frame would carry it.
fn encoded_len<T: serde::Serialize>(value: &T) -> usize {
    match proto::encode(value) {
        Ok(frame) => frame.len() - 4,
        Err(proto::CodecError::TooLarge(bytes)) => bytes,
        // Not encodable at all: the writer drops it whatever its size.
        Err(_) => 0,
    }
}

#[cfg(test)]
#[path = "snapshot_frame_tests.rs"]
mod frame_tests;
