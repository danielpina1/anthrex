//! Decision 56's protected-file bounce, moved out of `contract.rs` by milestone 9.13
//! (decision 3). Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

use super::globs::ProtectedMatcher;
use super::protected_lists::RUNS_LATER_PROTECTED;

/// Decision 56's rung-1 message (exact): one line per file. Milestone 9.13 decision 3:
/// the line says why the path is protected. A runs-later path (CI, git hooks, cargo or
/// toolchain configuration) gets its own line; the agent-config entries and any
/// user-added entry keep decision 56's line, byte for byte.
pub fn protected_file_message(files: &[String]) -> String {
    let runs_later: Vec<String> = RUNS_LATER_PROTECTED.iter().map(|s| s.to_string()).collect();
    let runs_later = ProtectedMatcher::new(&runs_later).ok();
    let mut lines = vec!["[anthrex] task_done rejected:".to_string()];
    lines.extend(files.iter().map(|path| {
        if runs_later.as_ref().is_some_and(|m| m.matches(path)) {
            format!(
                "{path} runs outside the run's sandbox (CI, git hooks, cargo or toolchain config); this task may change it only if its owns names it exactly"
            )
        } else {
            format!(
                "{path} configures or instructs future agents; this task may change it only if its owns names it exactly"
            )
        }
    }));
    lines
        .push("Revert it and call task_done again, or ask for the plan to be amended.".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Milestone 9.13 decision 3: a runs-later path gets its own line; the agent-config
    /// entries and a user-added one keep today's line, byte for byte.
    #[test]
    fn each_class_has_its_line() {
        let files: Vec<String> = [".github/workflows/ci.yml", "AGENTS.md", "ops/x.toml"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            protected_file_message(&files),
            "[anthrex] task_done rejected:\n\
             .github/workflows/ci.yml runs outside the run's sandbox (CI, git hooks, cargo or toolchain config); this task may change it only if its owns names it exactly\n\
             AGENTS.md configures or instructs future agents; this task may change it only if its owns names it exactly\n\
             ops/x.toml configures or instructs future agents; this task may change it only if its owns names it exactly\n\
             Revert it and call task_done again, or ask for the plan to be amended."
        );
    }
}
