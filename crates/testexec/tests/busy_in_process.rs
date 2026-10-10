//! The ETXTBSY race, reproduced deterministically with a script written in this process.
//!
//! This test is alone in its test binary on purpose. While it holds the script's write
//! descriptor, any fork another test thread takes (a `spawn`, a `write_executable`, a
//! [`StalledFork`]) inherits that descriptor too, and keeps the script busy after this
//! test's own fork is released: 51 of 200 runs failed that way when it shared `busy.rs`.
//! Here the only fork is the test's own [`StalledFork`]. Linux only: macOS does not
//! refuse to exec a file open for writing.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anthrex_testexec::StalledFork;

const SCRIPT: &str = "#!/bin/sh\nexit 0\n";

fn run(script: &Path) -> std::io::Result<std::process::ExitStatus> {
    Command::new(script).status()
}

/// The pattern every stand-in used before: written in this process, closed, chmod'ed and
/// renamed into place. A fork taken while the write descriptor was open keeps a copy of
/// it, on the same inode the rename put at `path`, so the exec is refused until that
/// child execs.
#[test]
fn a_script_written_in_process_is_busy_while_a_fork_holds_its_descriptor_renamed_or_not() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("stand-in.new");
    let script = dir.path().join("stand-in");
    let mut file = std::fs::File::create(&staged).unwrap();
    file.write_all(SCRIPT.as_bytes()).unwrap();
    let fork = StalledFork::start();
    drop(file);
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, &script).unwrap();

    let busy = run(&script).unwrap_err();
    assert_eq!(
        busy.kind(),
        std::io::ErrorKind::ExecutableFileBusy,
        "{busy}"
    );

    fork.release();
    assert!(
        run(&script).unwrap().success(),
        "runs once the fork has exec'd"
    );
}
