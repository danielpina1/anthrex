//! `write_warm_executable`: the script is exec'd once before it is returned, and that
//! run never reaches its body.

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use anthrex_testexec as testexec;

#[test]
fn the_warm_up_never_reaches_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let script = testexec::write_warm_executable(
        dir.path().join("stub"),
        format!("#!/bin/sh\necho body >> '{}'\nexit 3\n", marker.display()),
    );
    assert!(!marker.exists(), "the warm-up ran the body");
    let status = Command::new(&script).status().unwrap();
    assert_eq!(status.code(), Some(3));
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "body\n");
    let text = std::fs::read_to_string(&script).unwrap();
    let guard = format!(
        "#!/bin/sh\n[ -n \"${}\" ] && exit 0\n",
        testexec::WARM_UP_VAR
    );
    assert!(text.starts_with(&guard), "{text}");
}

#[test]
fn a_mode_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let script =
        testexec::write_warm_executable_mode(dir.path().join("stub"), "#!/bin/sh\nexit 0\n", 0o700);
    let mode = std::fs::metadata(&script).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
#[should_panic(expected = "#!/bin/sh")]
fn only_sh_scripts_are_warmed() {
    let dir = tempfile::tempdir().unwrap();
    testexec::write_warm_executable(dir.path().join("stub"), "#!/usr/bin/env python3\n");
}
