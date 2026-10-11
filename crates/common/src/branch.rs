//! The branch-name rules both the daemon and the client's new-window dialog apply
//! (design decision 8, rules 1 to 4; milestone 9.12 decisions 10 and 12). Pure.

/// Branch prefix that milestone 8 owns (design decision 8).
pub const RESERVED_BRANCH_PREFIX: &str = "anthrex/";

/// Design decision 8, rules 1 to 4. Pure. `crates/tui/src/dialog.rs` has its own copy
/// with the same messages (task M5.8).
pub fn check_branch_syntax(branch: &str) -> Result<(), String> {
    if branch.trim().is_empty() {
        return Err("branch name is required".to_string());
    }
    if branch.chars().any(char::is_whitespace) {
        return Err("branch name cannot contain spaces".to_string());
    }
    if branch.starts_with('-') {
        return Err("branch name cannot start with '-'".to_string());
    }
    if branch.starts_with(RESERVED_BRANCH_PREFIX) {
        return Err("branches under anthrex/ are reserved for orchestration runs".to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "branch_tests.rs"]
mod tests;
