//! Decision 56's protected-file bounce, moved out of `contract.rs` by milestone 9.13
//! (decision 3). Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

/// Decision 56's rung-1 message (exact): one line per file.
pub fn protected_file_message(files: &[String]) -> String {
    let mut lines = vec!["[anthrex] task_done rejected:".to_string()];
    lines.extend(files.iter().map(|path| {
        format!(
            "{path} configures or instructs future agents; this task may change it only if its owns names it exactly"
        )
    }));
    lines
        .push("Revert it and call task_done again, or ask for the plan to be amended.".to_string());
    lines.join("\n")
}
