//! Agent-facing message texts (the Interfaces "Contracts and message texts" table).
//! Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).
//!
//! M8a.6 creates this file with the two texts plan edits need, `answer_message` and
//! `amend_message`; M8a.11 adds the contracts, the prompts and the other messages.

use super::model::Task;

/// `[anthrex] Answer to your question: <text>`.
pub fn answer_message(text: &str) -> String {
    format!("[anthrex] Answer to your question: {text}")
}

/// `[anthrex] The task was amended.`, `Brief: <brief>`, `Acceptance criteria:`, one
/// `- <item>` per item and `Continue with the amended task.`, one per line: the amended
/// brief and acceptance criteria, delivered to a live worker (decision 13).
pub fn amend_message(task: &Task) -> String {
    let mut lines = vec![
        "[anthrex] The task was amended.".to_string(),
        format!("Brief: {}", task.spec.brief),
        "Acceptance criteria:".to_string(),
    ];
    lines.extend(task.spec.acceptance.iter().map(|item| format!("- {item}")));
    lines.push("Continue with the amended task.".to_string());
    lines.join("\n")
}
