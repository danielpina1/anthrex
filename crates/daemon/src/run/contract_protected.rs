//! Decision 56's protected-file bounce, moved out of `contract.rs` by milestone 9.13
//! (decision 3). Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

use super::gate_words::PinnedFile;
use super::globs::ProtectedMatcher;
use super::protected_lists::RUNS_LATER_PROTECTED;

/// Decision 56's rung-1 message (exact): one line per file. Milestone 9.13 decision 3:
/// the line says why the path is protected. A runs-later path (CI, git hooks, cargo or
/// toolchain configuration) gets its own line; the agent-config entries and any
/// user-added entry keep decision 56's line, byte for byte. A gate file (milestone 9.13
/// decisions 9 and 10, `Run.gate_files`) gets the gate line first (pre-flight ruling C2).
pub fn protected_file_message(files: &[String], gate_files: &[PinnedFile]) -> String {
    let runs_later: Vec<String> = RUNS_LATER_PROTECTED.iter().map(|s| s.to_string()).collect();
    let runs_later = ProtectedMatcher::new(&runs_later).ok();
    let mut lines = vec!["[anthrex] task_done rejected:".to_string()];
    lines.extend(files.iter().map(|path| {
        if gate_files.iter().any(|file| &file.path == path) {
            format!(
                "{path} is run by this run's own gate (check, setup or a tier command); this task may change it only if its owns names it exactly"
            )
        } else if runs_later.as_ref().is_some_and(|m| m.matches(path)) {
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
            protected_file_message(&files, &[]),
            "[anthrex] task_done rejected:\n\
             .github/workflows/ci.yml runs outside the run's sandbox (CI, git hooks, cargo or toolchain config); this task may change it only if its owns names it exactly\n\
             AGENTS.md configures or instructs future agents; this task may change it only if its owns names it exactly\n\
             ops/x.toml configures or instructs future agents; this task may change it only if its owns names it exactly\n\
             Revert it and call task_done again, or ask for the plan to be amended."
        );
    }

    /// Milestone 9.13 decision 3 (C2): a gate file gets the gate line, before the
    /// runs-later class is considered.
    #[test]
    fn a_gate_file_has_the_gate_line() {
        let gate_files = [PinnedFile {
            path: "scripts/check.sh".to_string(),
            blob: "a".repeat(40),
        }];
        let files: Vec<String> = ["scripts/check.sh", ".github/workflows/ci.yml"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            protected_file_message(&files, &gate_files),
            "[anthrex] task_done rejected:\n\
             scripts/check.sh is run by this run's own gate (check, setup or a tier command); this task may change it only if its owns names it exactly\n\
             .github/workflows/ci.yml runs outside the run's sandbox (CI, git hooks, cargo or toolchain config); this task may change it only if its owns names it exactly\n\
             Revert it and call task_done again, or ask for the plan to be amended."
        );
    }
}
