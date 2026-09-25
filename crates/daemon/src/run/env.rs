//! The profile's environment for engine commands and sessions, decision 7. Pure — no
//! `std::fs`, `std::process`, `std::thread`, `tokio` or `std::time::SystemTime` (design
//! decision 2).

use std::path::Path;

use super::model::Profile;

/// `profile.env` in key order, with `{worktree}` in each value replaced by `worktree`.
pub fn profile_env(profile: &Profile, worktree: &Path) -> Vec<(String, String)> {
    let path = worktree.display().to_string();
    profile
        .env
        .iter()
        .map(|(key, value)| (key.clone(), value.replace("{worktree}", &path)))
        .collect()
}
