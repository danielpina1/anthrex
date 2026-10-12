//! Milestone 9.13 decision 9: the tracked files a run's gate commands name, read at
//! run start. Blocking: [`read`] runs git, so `driver/build.rs` calls it on
//! `spawn_blocking`, never under the engine lock (AGENTS.md rule 2).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use crate::run::gate_words::PinnedFile;
use crate::run::git::{Git, os};

/// Decision 9: one `git --literal-pathspecs ls-tree -z <base_sha> -- <words…>` through
/// the run's [`Git`] wrapper (`--no-optional-locks`, a scrubbed environment), bounded
/// by `timeout`. An entry counts when it is a `blob` of mode `100644` or `100755`; a
/// tree, a symlink or a gitlink is not pinned. No words, no git call. A failure is the
/// caller's failed start (pre-flight ruling C26).
pub fn read(
    git: &OsStr,
    root: &Path,
    base_sha: &str,
    words: &[String],
    timeout: Duration,
) -> Result<Vec<PinnedFile>, String> {
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec![
        os("--literal-pathspecs"),
        os("ls-tree"),
        os("-z"),
        os(base_sha),
        os("--"),
    ];
    args.extend(words.iter().map(|w| os(w)));
    let listing = Git::new(git, timeout).ok(root, &args)?;
    Ok(parse(&listing))
}

/// `ls-tree -z` entries (`<mode> SP <type> SP <object> TAB <path> NUL`) that are
/// regular-file blobs, each once.
fn parse(listing: &str) -> Vec<PinnedFile> {
    let mut files: Vec<PinnedFile> = Vec::new();
    for entry in listing.split('\0').filter(|e| !e.is_empty()) {
        let Some((meta, path)) = entry.split_once('\t') else {
            continue;
        };
        let mut fields = meta.split(' ');
        let (Some(mode), Some(kind), Some(blob)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let regular = kind == "blob" && matches!(mode, "100644" | "100755");
        if regular && !files.iter().any(|f| f.path == path) {
            files.push(PinnedFile {
                path: path.to_string(),
                blob: blob.to_string(),
            });
        }
    }
    files
}

#[cfg(test)]
#[path = "gate_files_tests.rs"]
mod tests;
