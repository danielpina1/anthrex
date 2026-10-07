//! A worker's grant, by runtime and Codex sandbox dialect (task 5 of the Codex sandbox
//! profiles plan, and controller ruling R3). Pure: the caller supplies existence.

use std::path::{Path, PathBuf};

use crate::headless::codex_sandbox::CodexSandboxDialect;
use crate::run::git;
use crate::run::role_launch::protected_write_denials;

/// A worker's grant shape: the host's (on Linux the git directory whole, its protected
/// entries read-only) for Claude, and for Codex when its sandbox dialect can make a
/// path read-only inside a writable one; the exact files otherwise.
pub(super) fn grant_shape(claude: bool, dialect: CodexSandboxDialect) -> git::GrantShape {
    grant_shape_on(claude, dialect, git::GrantShape::host())
}

/// [`grant_shape`] on a host whose own shape is `host`.
pub(super) fn grant_shape_on(
    claude: bool,
    dialect: CodexSandboxDialect,
    host: git::GrantShape,
) -> git::GrantShape {
    if claude || dialect.expresses_read_only() {
        host
    } else {
        git::GrantShape::Files
    }
}

/// The checkout `cwd` spelled as the grant spells it (canonical), so every profile
/// entry joined onto it matches the grant's own: macOS `/tmp` is `/private/tmp`. Only
/// the checkout is resolved, never the tails joined onto it (a worker could swap a
/// tail for a link). The given path when it cannot be resolved. Blocking file work.
pub(super) fn canonical_checkout(cwd: &Path) -> PathBuf {
    std::fs::canonicalize(cwd).unwrap_or_else(|error| {
        tracing::debug!(cwd = %cwd.display(), %error, "checkout not canonicalized");
        cwd.to_path_buf()
    })
}

/// [`codex_read_only`] on the real filesystem: `cwd` canonicalized, existence by
/// `symlink_metadata`. Blocking file work.
pub(super) fn codex_read_only_on_disk(
    dialect: CodexSandboxDialect,
    cwd: &Path,
    owns: &[String],
    deny: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let cwd = canonical_checkout(cwd);
    codex_read_only(dialect, &cwd, owns, deny, |path| {
        std::fs::symlink_metadata(path).is_ok()
    })
}

/// A Codex worker's read-only entries in checkout `cwd` (ruling R3). Codex's legacy
/// `workspace-write` kept `.git` and `.codex` under a writable root read-only on its
/// own; a profile extending `:read-only` with a `"write"` entry for the checkout does
/// not, so they are named here: the protected agent-config paths a Claude worker is
/// denied (`protected_write_denials`, owned literals left out) and the checkout's
/// `.git`, each only where `exists` finds it (Claude's sandbox skips a missing path
/// too, and a profile entry for a missing one is unverified Codex behaviour), then the
/// grant's own denials `deny` (made to exist by the grant). Each path once, in its
/// first place. Empty under a dialect that cannot express a read-only path inside a
/// writable one: there the legacy plan would fall back to a read-only session.
pub(super) fn codex_read_only(
    dialect: CodexSandboxDialect,
    cwd: &Path,
    owns: &[String],
    deny: Vec<PathBuf>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    if !dialect.expresses_read_only() {
        return Vec::new();
    }
    let protected = protected_write_denials(cwd, owns)
        .into_iter()
        .chain(std::iter::once(cwd.join(".git")))
        .filter(|path| exists(path));
    let mut read_only: Vec<PathBuf> = Vec::new();
    for path in protected.chain(deny) {
        if !read_only.contains(&path) {
            read_only.push(path);
        }
    }
    read_only
}

#[cfg(test)]
#[path = "worker_grant_tests.rs"]
mod tests;
