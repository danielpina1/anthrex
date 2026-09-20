//! `enabled_from_env` (design decision 21), alone in its own test binary.
//!
//! It lives here rather than beside the rest of the registry's tests because it mutates
//! the process environment, and the edition-2024 hazard that makes `std::env::set_var`
//! unsafe is not writer-against-writer — a mutex would cover that. It is a writer
//! against any concurrent *reader* of `environ`: `getenv` from any other thread, and
//! every process spawn, which reads the whole block to build the child's environment.
//! `environ` can be reallocated by a write, so a reader running beside it is a data
//! race and therefore undefined behaviour, no matter that the reader wants a different
//! variable entirely.
//!
//! libtest runs a binary's tests on threads of one process, so the only way to have no
//! concurrent reader is to have no other test in the binary. `crates/daemon/tests/
//! git_registry.rs` — where this test used to live — spawns `git` from
//! `a_real_write_triggers_a_probe` and its helpers, on other libtest threads, which is
//! exactly the reader this needs to exclude. Keep this file to this one test.

use daemon::git::enabled_from_env;

#[test]
fn enabled_from_env_reads_anthrex_git() {
    // SAFETY: this is the only test in this binary, so no other libtest thread exists
    // to read `environ` while these writes reallocate it (see the module docs). The
    // test restores the variable it found, leaving the process as it started.
    let restore = std::env::var_os("ANTHREX_GIT");

    unsafe {
        std::env::remove_var("ANTHREX_GIT");
    }
    assert!(enabled_from_env(), "unset leaves git on");

    for (value, expected) in [("off", false), ("0", false), ("", true), ("on", true)] {
        unsafe {
            std::env::set_var("ANTHREX_GIT", value);
        }
        assert_eq!(enabled_from_env(), expected, "ANTHREX_GIT={value:?}");
    }

    unsafe {
        match restore {
            Some(value) => std::env::set_var("ANTHREX_GIT", value),
            None => std::env::remove_var("ANTHREX_GIT"),
        }
    }
}
