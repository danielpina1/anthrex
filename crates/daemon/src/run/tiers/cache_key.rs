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

/// Decision 29: the environment variables that change what a toolchain produces. A
/// leased `CARGO_TARGET_DIR` and the slot variables are never inputs, so none is here.
pub const TOOLCHAIN_ENV_VARS: [&str; 5] = [
    "RUSTFLAGS",
    "RUSTDOCFLAGS",
    "RUSTC_WRAPPER",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_TARGET",
];

/// The key's toolchain part with the step's effective `PATH` and the listed variables
/// in it (ruling C-13 (5), decision 29): `<toolchain> path:<fnv>`, then ` env:<fnv>`
/// over `NAME=value\n` for each set variable. The daemon's own environment is read
/// here.
pub fn with_env(toolchain: &str, env: &[(String, String)]) -> String {
    with_env_from(toolchain, env, &|name| std::env::var(name).ok())
}

/// The pure core of [`with_env`]: `daemon` looks a variable up in the daemon's own
/// environment. The step env's last value wins over the daemon's.
pub fn with_env_from(
    toolchain: &str,
    env: &[(String, String)],
    daemon: &dyn Fn(&str) -> Option<String>,
) -> String {
    let lookup = |name: &str| {
        env.iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .or_else(|| daemon(name))
    };
    let mut hash = crate::profile::store::Fnv1a64::new();
    hash.update(lookup("PATH").unwrap_or_default().as_bytes());
    let mut out = format!("{toolchain} path:{:016x}", hash.0);
    let mut vars = String::new();
    for name in TOOLCHAIN_ENV_VARS {
        if let Some(value) = lookup(name) {
            vars.push_str(&format!("{name}={value}\n"));
        }
    }
    if !vars.is_empty() {
        let mut hash = crate::profile::store::Fnv1a64::new();
        hash.update(vars.as_bytes());
        out.push_str(&format!(" env:{:016x}", hash.0));
    }
    out
}
