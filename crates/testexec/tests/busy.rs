//! The ETXTBSY race, reproduced deterministically: a [`StalledFork`] is a sibling
//! test's `spawn` caught between fork and exec, held for as long as the test needs.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anthrex_testexec::{StalledFork, write_executable, write_executable_with};

const SCRIPT: &str = "#!/bin/sh\nexit 0\n";

fn run(script: &Path) -> std::io::Result<std::process::ExitStatus> {
    Command::new(script).status()
}

/// The same fork, taken while [`write_executable`] is writing, holds nothing of the
/// script: the only write descriptor ever opened on it lived in the writer child.
#[test]
fn a_script_from_write_executable_runs_while_a_fork_taken_mid_write_is_still_held() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("stand-in");
    let mut fork = None;
    write_executable_with(&script, SCRIPT, 0o755, || fork = Some(StalledFork::start()));

    assert!(run(&script).unwrap().success());
    fork.unwrap().release();
}

#[test]
fn write_executable_sets_the_mode_and_the_bytes_and_leaves_no_staging_file() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_executable(dir.path().join("stand-in"), "#!/bin/sh\necho 'it ran'\n");
    assert_eq!(
        std::fs::metadata(&script).unwrap().permissions().mode() & 0o7777,
        0o755
    );
    let output = Command::new(&script).output().unwrap();
    assert_eq!(output.stdout, b"it ran\n");
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from("stand-in")]);
}

#[test]
fn write_executable_mode_keeps_an_owner_only_mode_and_replaces_an_existing_script() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_executable(dir.path().join("stand-in"), "#!/bin/sh\necho first\n");
    anthrex_testexec::write_executable_mode(&path, "#!/bin/sh\necho second\n", 0o700);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o700
    );
    assert_eq!(Command::new(&path).output().unwrap().stdout, b"second\n");
}
