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

/// Decision 35 and ruling Q4: the reviewer's diff, and decision 30's hand-over diff, are
/// clamped to this many bytes.
pub const REVIEW_DIFF_MAX: usize = 16 * 1024;

/// The line [`clamp_diff`] puts where it cut the middle out of a diff.
pub const DIFF_CUT_MARKER: &str = "\n[anthrex: the middle of this diff was cut to fit]\n";

/// A head-and-tail clamp on character boundaries: `text` itself when it is at most
/// `max` bytes, else its first part, [`DIFF_CUT_MARKER`] and its last part, together at
/// most `max` bytes and never more than 3 bytes short of it (the most a UTF-8 cut can
/// cost, since the tail takes whatever the head's cut left over).
pub fn clamp_diff(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    if max <= DIFF_CUT_MARKER.len() {
        return text[..floor_boundary(text, max)].to_string();
    }
    let budget = max - DIFF_CUT_MARKER.len();
    let head_end = floor_boundary(text, budget / 2);
    let tail_len = budget - head_end;
    let tail_start = ceil_boundary(text, text.len() - tail_len);
    let mut out = String::with_capacity(max);
    out.push_str(&text[..head_end]);
    out.push_str(DIFF_CUT_MARKER);
    out.push_str(&text[tail_start..]);
    out
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every cut position modulo a character: `k` ASCII bytes shift a body of 3-byte
    /// (`世`) or 4-byte (`𝄞`) characters, so over `k` in 0..=3 each cut lands on every
    /// offset inside a character.
    #[test]
    fn clamp_diff_cuts_on_character_boundaries_at_every_offset() {
        for body in ["世", "𝄞", "a世𝄞"] {
            for k in 0..=3 {
                let text = format!("{}{}", "a".repeat(k), body.repeat(20_000));
                for max in [REVIEW_DIFF_MAX, 1000, 1001, 1002, 1003] {
                    let out = clamp_diff(&text, max);
                    assert!(out.len() <= max, "{body} k={k} max={max}: {}", out.len());
                    assert!(
                        out.len() >= max - 3,
                        "{body} k={k} max={max}: {}",
                        out.len()
                    );
                    assert_eq!(out.matches(DIFF_CUT_MARKER).count(), 1);
                    let (head, tail) = out.split_once(DIFF_CUT_MARKER).unwrap();
                    assert!(text.starts_with(head), "{body} k={k} max={max}");
                    assert!(text.ends_with(tail), "{body} k={k} max={max}");
                }
            }
        }
    }

    #[test]
    fn clamp_diff_leaves_text_within_the_limit_alone() {
        let text = "世".repeat(10);
        assert_eq!(clamp_diff(&text, 30), text);
        assert_eq!(clamp_diff(&text, 31), text);
        let cut = clamp_diff(&text, 29);
        assert!(cut.len() <= 29, "{cut:?}");
    }

    #[test]
    fn clamp_diff_below_the_marker_keeps_a_head_only() {
        let text = "世".repeat(100);
        let out = clamp_diff(&text, 10);
        assert_eq!(out, "世世世");
    }
}
