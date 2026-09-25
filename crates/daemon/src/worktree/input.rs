//! Standard input for the git commands that need it (`update-index --index-info`,
//! M8a final fix batch F1, fix round 5: see [`super::run_git_with_input`]; and the
//! import of a worker's objects, `index-pack --stdin` fed a pack file, final fix batch
//! F1b: [`run_git_with_stdin_file`]).

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::Instant;

use super::{Capture, GitOutput, MAX_OUTPUT_BYTES, WorktreeError, run_git_capturing};

/// As [`super::run_git_with_input`], with an open file as git's standard input (read
/// from its current position to its end): a pack too large to hold in memory.
pub fn run_git_with_stdin_file(
    git: &OsStr,
    dir: &Path,
    args: &[&OsStr],
    deadline: Instant,
    file: &std::fs::File,
) -> Result<GitOutput, WorktreeError> {
    run_git_capturing(
        git,
        dir,
        args,
        deadline,
        Capture::Capped(MAX_OUTPUT_BYTES),
        Some(file),
    )
    .map(|(output, _)| output)
}

/// `input` in a new file under the temporary directory, created exclusively (`0600`),
/// unlinked at once, and rewound: only the returned handle reaches it.
pub(super) fn input_file(input: &[u8]) -> io::Result<std::fs::File> {
    use std::io::{Seek, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    loop {
        let path = dir.join(format!(
            "anthrex-git-input-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let opened = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path);
        match opened {
            Ok(mut file) => {
                let _ = std::fs::remove_file(&path);
                file.write_all(input)?;
                file.rewind()?;
                return Ok(file);
            }
            // Left by an earlier daemon with this pid: take the next name.
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::time::{Duration, Instant};

    /// The bytes reach git's stdin whole: `hash-object --stdin` names the blob.
    #[test]
    fn git_reads_the_input_as_its_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let args = [OsStr::new("hash-object"), OsStr::new("--stdin")];
        let deadline = Instant::now() + Duration::from_secs(30);
        let output = super::super::run_git_with_input(
            OsStr::new("git"),
            dir.path(),
            &args,
            deadline,
            b"hello\n",
        )
        .unwrap();
        assert!(output.success, "{output:?}");
        assert_eq!(
            output.stdout.trim(),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
    }
}
