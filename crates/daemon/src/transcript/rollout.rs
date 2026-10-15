//! Milestone 9.16 decision 9: where a Codex session's rollout is. Blocking; called only
//! inside `spawn_blocking`.

use std::ffi::OsString;
use std::path::PathBuf;

/// Ruling T8-4: the folder Codex saves its sessions in, `$CODEX_HOME/sessions`, else
/// `~/.codex/sessions`, from the daemon's environment (`var`).
pub fn codex_sessions_dir(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |key: &str| var(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    match set("CODEX_HOME") {
        Some(home) => Some(home.join("sessions")),
        None => set("HOME").map(|home| home.join(".codex").join("sessions")),
    }
}
