//! `worktree::run_git`'s `--no-optional-locks` placement and environment scrubbing
//! (design decisions 1 and 2), alone in its own test binary.
//!
//! It lives here rather than inside `worktree.rs`'s own `#[cfg(test)] mod tests`, for
//! the same reason `crates/daemon/tests/git_env.rs` isolates its one env-mutating test —
//! but note what the actual hazard is, because `git_env.rs`'s own comment names it
//! imprecisely. The danger is not two writers racing each other; a `Mutex` around the
//! mutation would close that, trivially. The danger is a writer racing a *reader*:
//! `std::env::set_var` can reallocate the process's `environ` block, and building a
//! child process reads that whole block (unless the `Command` has been given a fully
//! custom environment) to hand to the child — that read is not gated by any lock this
//! crate controls. `worktree.rs`'s own test module runs
//! `a_missing_git_is_reported_as_such`, which calls `run_git` and therefore spawns a
//! process, on another libtest thread of that binary. A `Mutex` taken only around this
//! test's own `set_var` calls does nothing to stop that: no spawn anywhere else in the
//! binary ever takes it, so the two threads can still overlap. That overlap is
//! undefined behaviour in edition 2024 (the reason `std::env::set_var` is `unsafe` at
//! all), and its signature is an unreproducible crash — exactly the class of bug
//! AGENTS.md's "facts learned the hard way" section already records once, for a
//! different root cause.
//!
//! libtest runs a binary's tests on threads of one process, so the only way to
//! guarantee there is no concurrent reader is to have no other test — spawning or not —
//! in the binary. Keep this file to this one test.

use daemon::worktree::run_git;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let script = dir.join(name);
    fs::write(&script, body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    script
}

#[test]
fn the_git_helper_passes_no_optional_locks_and_scrubs_the_environment() {
    let scripts = tempdir().unwrap();
    let argv_log = scripts.path().join("argv.log");
    let env_log = scripts.path().join("env.log");
    let script = write_script(
        scripts.path(),
        "recording-git",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nenv >> '{}'\nexit 0\n",
            argv_log.display(),
            env_log.display()
        ),
    );

    let scrubbed = [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_PREFIX",
    ];
    let previous: Vec<(&str, Option<OsString>)> = scrubbed
        .iter()
        .map(|key| (*key, std::env::var_os(key)))
        .collect();
    // SAFETY: this is the only test in this binary (see the module doc comment above),
    // so there is no concurrent *reader* of `environ` — no other thread here ever
    // spawns a process or otherwise reads the environment — for these writes to race.
    // That absence of a reader is what makes this sound, not any lock: there is no
    // second writer to serialize against either.
    unsafe {
        for key in scrubbed {
            std::env::set_var(key, "leak-marker");
        }
    }

    let dir = tempdir().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = run_git(script.as_os_str(), dir.path(), &["status"], deadline);

    // SAFETY: same as above.
    unsafe {
        for (key, value) in &previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    let output = result.expect("the recording script always exits zero");
    assert!(output.success);

    let recorded_argv = fs::read_to_string(&argv_log).unwrap();
    let line = recorded_argv.lines().next().expect("one invocation");
    let parts: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(parts[0], "-C", "{parts:?}");
    assert_eq!(parts[1], dir.path().to_str().unwrap(), "{parts:?}");
    assert_eq!(parts[2], "--no-optional-locks", "{parts:?}");
    assert_eq!(parts[3], "status", "{parts:?}");

    let recorded_env = fs::read_to_string(&env_log).unwrap();
    for key in scrubbed {
        assert!(
            !recorded_env
                .lines()
                .any(|line| line.starts_with(&format!("{key}="))),
            "{key} leaked into the child environment:\n{recorded_env}"
        );
    }
}
