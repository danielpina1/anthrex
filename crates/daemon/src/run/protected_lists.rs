//! Milestone 9.13 decision 1: decision 56's built-in protected paths, in two classes.
//! Configuration and the plan add to them; neither can remove one. Pure — no
//! `std::fs`, `std::process`, `std::thread`, `tokio` or `std::time::SystemTime`
//! (design decision 2).

/// Files that configure or instruct future agents (M8a decision 56; milestone 9.11
/// decision 33 added the last three: the agents' override, local and shared files).
pub const AGENT_CONFIG_PROTECTED: &[&str] = &[
    ".claude/**",
    ".mcp.json",
    ".codex/**",
    "**/CLAUDE.md",
    "**/AGENTS.md",
    "**/AGENTS.override.md",
    "**/CLAUDE.local.md",
    ".agents/**",
];

/// Files that run outside the run's sandbox on a later command: CI, git hooks, cargo
/// and toolchain configuration (milestone 9.13 decision 1, NA §2 A1).
pub const RUNS_LATER_PROTECTED: &[&str] = &[
    ".github/workflows/**",
    ".github/actions/**",
    ".gitlab-ci.yml",
    ".circleci/**",
    ".buildkite/**",
    "Jenkinsfile",
    "azure-pipelines.yml",
    ".githooks/**",
    ".husky/**",
    ".pre-commit-config.yaml",
    "**/.cargo/config",
    "**/.cargo/config.toml",
    "**/rust-toolchain",
    "**/rust-toolchain.toml",
];

/// Every built-in protected entry: the agent-config list, then the runs-later list.
pub fn builtin_protected() -> Vec<String> {
    AGENT_CONFIG_PROTECTED
        .iter()
        .chain(RUNS_LATER_PROTECTED)
        .map(|s| s.to_string())
        .collect()
}
