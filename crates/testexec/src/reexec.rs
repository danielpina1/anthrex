//! Helpers for a test that re-executes its own binary to run one other test.
//!
//! `--exact` matches the whole libtest name, and a libtest child that matches nothing
//! runs zero tests and exits 0, so a parent that only checks the exit status passes
//! without the child's body ever running. [`libtest_name`] builds the name, and
//! [`assert_one_test_ran`] fails the parent when the child did not run exactly one test.

use std::process::Output;

/// The libtest name of a test function: `module_path` without its first segment, then `name`.
/// `libtest_name("it::manager::lifecycle", "f")` is `"manager::lifecycle::f"`. A
/// `module_path` of one segment (a file binary's root) gives `name` unchanged.
pub fn libtest_name(module_path: &str, name: &str) -> String {
    match module_path.split_once("::") {
        Some((_, module)) => format!("{module}::{name}"),
        None => name.to_string(),
    }
}

/// Panics unless a libtest child's stdout contains the line `running 1 test` and a line
/// starting `test result: ok. 1 passed`: one test ran, and it passed.
pub fn assert_one_test_ran(output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.lines().any(|line| line == "running 1 test")
            && stdout
                .lines()
                .any(|line| line.starts_with("test result: ok. 1 passed")),
        "the re-executed test binary did not run exactly one test (a renamed test?); its stdout was:\n{stdout}"
    );
}
