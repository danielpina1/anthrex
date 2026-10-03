//! Milestone 9.5 decision 40 (FU-F27, review ruling I11): a Codex orchestrator that
//! asks through Codex's own question tool shows `? <n> question` in its footer, while
//! its window may be `Working`, `Idle` or `Done`. The manager scans the screen's last
//! rows on every tick (`WindowManager::tick`) and raises `Attention` on a new match
//! (`StatusEvent::CodexQuestion`). Pure: text in, a verdict out.
//!
//! The pattern comes from the user's try-out (`Queued follow-up inputs · ? 1 question ·
//! shift+← to answer`); the fixture `tests/fixtures/screens/codex-question-footer.txt`
//! is hand-written from it until a real capture replaces it.

use std::sync::LazyLock;

use regex::Regex;

/// How many of the screen's last non-empty rows are its footer.
pub const FOOTER_ROWS: usize = 3;

static QUESTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\?\s+(\d+)\s+questions?\b").expect("a valid pattern"));

/// Whether any of `rows` is Codex's queued-question footer.
pub fn codex_question_footer(rows: &[&str]) -> bool {
    question_count(rows).is_some()
}

/// How many questions the footer in `rows` counts (the highest, if several match);
/// `None` with no footer, or one counting none.
pub fn question_count(rows: &[&str]) -> Option<u32> {
    (rows.iter())
        .flat_map(|row| QUESTION.captures_iter(row))
        .filter_map(|c| c[1].parse::<u32>().ok())
        .filter(|n| *n > 0)
        .max()
}

/// The screen text's last [`FOOTER_ROWS`] non-empty rows, top to bottom, trimmed.
pub fn footer_rows(screen: &str) -> Vec<&str> {
    let mut rows: Vec<&str> = (screen.lines().rev())
        .map(str::trim)
        .filter(|row| !row.is_empty())
        .take(FOOTER_ROWS)
        .collect();
    rows.reverse();
    rows
}

/// Whether `screen` (a window's `Window::screen_text`) ends with the footer.
pub fn screen_asks(screen: &str) -> bool {
    screen_questions(screen).is_some()
}

/// [`question_count`] of `screen`'s footer rows.
pub fn screen_questions(screen: &str) -> Option<u32> {
    question_count(&footer_rows(screen))
}

#[cfg(test)]
#[path = "status_codex_rig.rs"]
pub(crate) mod rig;

#[cfg(test)]
#[path = "status_codex_tests.rs"]
mod tests;
