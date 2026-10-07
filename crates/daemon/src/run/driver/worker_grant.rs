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
    if claude || dialect.expresses_read_only() {
        git::GrantShape::host()
    } else {
        git::GrantShape::Files
    }
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
