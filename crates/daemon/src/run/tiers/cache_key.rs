//! Decision 30: the result cache's key. Pure. A lookup compares every field; it never
//! trusts a hash alone.

use serde::{Deserialize, Serialize};

use super::{CacheCtx, Scope};

/// `(tree id, scope, command after substitution, affected, profile hash, toolchain
/// id)`. The scope, not the tier, so tier 1's green result answers tier 2 on the same
/// tree (TT §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheKey {
    pub tree: String,
    pub scope: Scope,
    pub command: String,
    /// The sorted module names joined by `,`, `full:<reason>`, or `-` for a build step.
    pub affected: String,
    pub profile_hash: String,
    pub toolchain: String,
}

/// A step's key: `command` and `affected_key` as the step carries them (`Step`).
pub fn key(
    tree: &str,
    scope: Scope,
    command: &str,
    affected_key: &str,
    ctx: &CacheCtx,
) -> CacheKey {
    CacheKey {
        tree: tree.to_string(),
        scope,
        command: command.to_string(),
        affected: affected_key.to_string(),
        profile_hash: ctx.profile_hash.clone(),
        toolchain: ctx.toolchain.clone(),
    }
}

/// The key's toolchain part with the step's effective `PATH` in it (ruling C-13 (5)):
/// a program found on another `PATH` is another toolchain.
pub fn with_path(toolchain: &str, path: &str) -> String {
    let mut hash = crate::profile::store::Fnv1a64::new();
    hash.update(path.as_bytes());
    format!("{toolchain} path:{:016x}", hash.0)
}
