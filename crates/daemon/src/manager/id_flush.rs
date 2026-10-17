//! A spent window id reaches `state.json` before the window is published.
//!
//! `restore` rebuilds `next_id` from the saved file alone. The persister writes after a
//! debounce (`state::SAVE_DEBOUNCE`, up to `state::SAVE_MAX_DELAY`), so an id published
//! inside that window and lost to a crash would be handed out again by the next daemon,
//! and a client that kept its focus by id would send the user's keys to whichever agent
//! got it. Each create therefore asks the persister for an immediate write between
//! spending the id (phase A) and publishing the window, and waits for it with no lock
//! held.

use super::WindowManager;

impl WindowManager {
    /// Installs the persister's flush request (`state::spawn_persister`). A later
    /// persister replaces an earlier one's.
    pub fn set_state_flusher(&self, handle: crate::state::FlushHandle) {
        *crate::lock(&self.state_flusher) = Some(handle);
    }

    /// The id the next window will get. Under the lock, no I/O.
    pub fn next_id(&self) -> u32 {
        crate::lock(&self.inner).next_id
    }

    /// Waits until `state.json` records every id spent so far, `id` included. Called with
    /// no lock held: the persister takes the manager lock for its snapshot. A manager no
    /// persister runs for (most tests) has nothing to wait for.
    ///
    /// A failed write fails the create: going on would publish an id the next daemon may
    /// reissue, which is the hazard this exists to close.
    pub(super) async fn record_spent_id(&self, id: u32) -> anyhow::Result<()> {
        let Some(flusher) = crate::lock(&self.state_flusher).clone() else {
            return Ok(());
        };
        flusher.flush().await.map_err(|error| {
            anyhow::anyhow!("window {id} was not opened: its id could not be saved: {error}")
        })
    }
}
