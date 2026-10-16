//! Milestone 9.16 decision 16: the hold's file, `<data_dir>/brakes.json`
//! (`{"v": 1, "hold": {"since", "by"}}` or `{"v": 1, "hold": null}`).
//!
//! [`save`] writes a temp file in the same directory, mode 0600, `fsync`s it and
//! renames it over the file; [`load`] reads it once at the service's start. Both run
//! their blocking half on `spawn_blocking` within a wait ([`BRAKES_IO_WAIT`] in
//! production), never under a lock (AGENTS.md rule 2). The load fails closed: a file
//! that does not parse or has another `v` is moved aside to
//! `brakes.json.corrupt-<unix>` and holds with [`CORRUPT_BY`], a hold written back to a
//! fresh file so a restart stays held (task 8's ruling); a read that fails or
//! does not finish in time holds with [`UNREADABLE_BY`] and leaves the file as it is.
//! Only a missing file is off. Never in `state.json`.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::brakes::DispatchHold;
use serde::{Deserialize, Serialize};

pub const BRAKES_FILE: &str = "brakes.json";
/// Decision 16: every read and write of [`BRAKES_FILE`] finishes within this.
pub const BRAKES_IO_WAIT: Duration = Duration::from_secs(5);
/// Decision 15: who holds when the file did not parse.
pub const CORRUPT_BY: &str = "corrupt brakes.json";
/// Decision 15: who holds when the file could not be read in time.
pub const UNREADABLE_BY: &str = "unreadable brakes.json";
/// The file's only version.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct BrakesFile {
    v: u32,
    #[serde(default)]
    hold: Option<DispatchHold>,
}

/// `f` on `spawn_blocking`, given at most `wait`; `Err` names a panic or the timeout.
async fn within<T: Send + 'static>(
    wait: Duration,
    f: impl FnOnce() -> std::io::Result<T> + Send + 'static,
) -> Result<T, String> {
    match tokio::time::timeout(wait, tokio::task::spawn_blocking(f)).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(error))) => Err(error.to_string()),
        Ok(Err(error)) => Err(format!("a blocking step did not finish: {error}")),
        Err(_) => Err(format!("it did not finish within {} ms", wait.as_millis())),
    }
}

/// Writes `hold` to `<data_dir>/brakes.json` (decision 16); `Err` is why it was not.
pub async fn save(
    data_dir: &Path,
    hold: Option<DispatchHold>,
    wait: Duration,
) -> Result<(), String> {
    let text =
        serde_json::to_vec(&BrakesFile { v: VERSION, hold }).map_err(|error| error.to_string())?;
    let dir = data_dir.to_path_buf();
    within(wait, move || write_atomic(&dir, &text)).await
}

/// The temp file, `fsync`, rename; a temp file left by a failure is removed.
fn write_atomic(dir: &Path, text: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{BRAKES_FILE}.tmp"));
    // Only one save runs at a time (the service's hold lock); a leftover of a crash
    // is replaced, so the new file is created with its mode.
    let _ = std::fs::remove_file(&tmp);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(text)?;
        file.sync_all()?;
        std::fs::rename(&tmp, dir.join(BRAKES_FILE))
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return written;
    }
    // The rename's own durability; a directory that cannot be synced is not an error.
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// The hold `<data_dir>/brakes.json` holds (decision 16), read within `wait`; `now`
/// is a fail-closed hold's `since`.
pub async fn load(data_dir: &Path, wait: Duration, now: u64) -> Option<DispatchHold> {
    let path = data_dir.join(BRAKES_FILE);
    let read = path.clone();
    // Only the read is on the thread: one that outlives `wait` renames nothing.
    let bytes = match within(wait, move || match std::fs::read(&read) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    })
    .await
    {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "brakes.json could not be read; dispatch is held");
            return Some(fail_closed(UNREADABLE_BY, now));
        }
    };
    match serde_json::from_slice::<BrakesFile>(&bytes) {
        Ok(file) if file.v == VERSION => file.hold,
        parsed => {
            let why = match parsed {
                Ok(file) => format!("version {} is not {VERSION}", file.v),
                Err(error) => error.to_string(),
            };
            let aside = corrupt_path(data_dir, now);
            let to = aside.clone();
            let moved = within(wait, move || std::fs::rename(&path, &to)).await;
            let hold = fail_closed(CORRUPT_BY, now);
            match moved {
                Ok(()) => {
                    tracing::warn!(%why, aside = %aside.display(), "brakes.json is corrupt; moved aside, dispatch is held");
                    // Task 8's ruling: the fail-closed hold is written back, so a restart
                    // stays held until the user lifts it. A file that could not be moved
                    // aside is left for the next start, which fails closed on it again.
                    if let Err(error) = save(data_dir, Some(hold.clone()), wait).await {
                        tracing::warn!(%error, "the fail-closed hold was not saved; a restart starts unheld");
                    }
                }
                Err(error) => {
                    tracing::warn!(%why, %error, "brakes.json is corrupt and could not be moved aside; dispatch is held")
                }
            }
            Some(hold)
        }
    }
}

fn fail_closed(by: &str, now: u64) -> DispatchHold {
    DispatchHold {
        since: now,
        by: by.to_string(),
    }
}

/// `brakes.json.corrupt-<unix>`.
fn corrupt_path(data_dir: &Path, now: u64) -> PathBuf {
    data_dir.join(format!("{BRAKES_FILE}.corrupt-{now}"))
}

#[cfg(test)]
#[path = "brakes_io_tests.rs"]
mod tests;
