//! The self re-exec helpers: the name a re-executed test binary is asked to run, and
//! the check that the child ran exactly that one test.

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitStatus, Output};

use anthrex_testexec::{assert_one_test_ran, libtest_name};

const MARKER: &str = "reexec-child-marker-4f1c";

fn output_with_stdout(stdout: &str) -> Output {
    Output {
        status: ExitStatus::from_raw(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

#[test]
fn libtest_name_drops_the_crate_segment() {
    assert_eq!(
        libtest_name("it::manager::lifecycle", "f"),
        "manager::lifecycle::f"
    );
    // A file binary's tests sit at its root.
    assert_eq!(libtest_name("profile_verify", "child"), "child");
}

#[test]
fn one_test_ran_is_accepted() {
    assert_one_test_ran(&output_with_stdout(
        "\nrunning 1 test\ntest x ... ok\n\ntest result: ok. 1 passed\n",
    ));
}

#[test]
#[should_panic(expected = "did not run exactly one test")]
fn zero_tests_ran_is_refused() {
    assert_one_test_ran(&output_with_stdout(
        "\nrunning 0 tests\n\ntest result: ok. 0 passed\n",
    ));
}

/// Only the re-executed child runs this (`--ignored`); the parent below names it.
#[test]
#[ignore = "run only by the_child_really_runs, through a re-executed test binary"]
fn child_marker() {
    println!("{MARKER}");
}

#[test]
fn the_child_really_runs() {
    let name = libtest_name(module_path!(), "child_marker");
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--ignored", "--nocapture"])
        .output()
        .unwrap();
    assert_one_test_ran(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(MARKER), "{stdout}");
}
