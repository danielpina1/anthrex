//! Stand-ins whose first exec is paid before a test's clock starts.
//!
//! On macOS the first exec of a freshly written executable waits for a security
//! assessment before its first line runs: ~0.44 s alone, serialised across processes to
//! seconds under a parallel run, and measured up to 10 s on a loaded machine
//! (`docs/timing-budgets.md`, "First exec of a freshly written executable"). A test that
//! writes a stub and then waits a bounded time for the stub's first run pays that inside
//! its bound. [`write_warm_executable`] execs the script once before returning; a guard
//! after the shebang ends that run before the script's body, so a body with side effects
//! (a marker, an argv log, a counter) records nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Set only on the warm-up run: the guard line exits 0 before the body.
pub const WARM_UP_VAR: &str = "ANTHREX_TEST_WARM_UP";

/// Writes a `#!/bin/sh` script with mode 0755 and execs it once, as
/// [`write_warm_executable_mode`].
pub fn write_warm_executable(path: impl AsRef<Path>, body: impl AsRef<str>) -> PathBuf {
    write_warm_executable_mode(path, body, 0o755)
}

/// Writes `body`, a `#!/bin/sh` script, to `path` with `mode` (as
/// [`crate::write_executable_mode`]) behind a warm-up guard, and execs it once with
/// [`WARM_UP_VAR`] set so the file's first exec is this one. Panics when `body` is not a
/// `#!/bin/sh` script or the warm-up does not exit 0: this is test support.
///
/// The warm-up is unbounded: the cost it pays has no bound worth asserting, and a test
/// that hangs here names this helper in its stack.
pub fn write_warm_executable_mode(
    path: impl AsRef<Path>,
    body: impl AsRef<str>,
    mode: u32,
) -> PathBuf {
    let path = path.as_ref();
    let rest = body
        .as_ref()
        .strip_prefix("#!/bin/sh\n")
        .unwrap_or_else(|| panic!("{} must be a #!/bin/sh script", path.display()));
    let guarded = format!("#!/bin/sh\n[ -n \"${WARM_UP_VAR}\" ] && exit 0\n{rest}");
    crate::write_executable_mode(path, guarded, mode);
    let warm = Command::new(path)
        .env(WARM_UP_VAR, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|e| panic!("cannot warm up {}: {e}", path.display()));
    assert!(
        warm.success(),
        "warming up {} failed: {warm}",
        path.display()
    );
    path.to_path_buf()
}
