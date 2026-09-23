//! Decision 29's message clamp and turn join. Pure — no `std::fs`, `std::process`,
//! `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2).

use super::contract::clamp_with;
use super::model::Outgoing;

/// The most one delivered turn may carry.
pub const MESSAGE_MAX_BYTES: usize = 32 * 1024;
/// A failed delivery is retried at the first `Tick` this many seconds later.
pub const DELIVERY_RETRY_SECS: u64 = 5;
/// Consecutive failed deliveries that block the task as `blocked(environment)`.
pub const DELIVERY_MAX_FAILURES: u8 = 3;

/// Lines of a check's tail a bounce message carries (decision 34).
pub const CHECK_SUMMARY_LINES: usize = 40;

/// The last [`CHECK_SUMMARY_LINES`] lines of `tail`, what a bounce carries (decision 34;
/// the decider's summary is M8b).
pub fn summary(tail: &str) -> String {
    let lines: Vec<&str> = tail.split('\n').collect();
    let start = lines.len().saturating_sub(CHECK_SUMMARY_LINES);
    lines[start..].join("\n")
}

/// The line [`clamp`] puts where it cut the middle out of a message.
pub const MESSAGE_CUT_MARKER: &str = "\n[anthrex: the middle of this message was cut to fit]\n";

/// `text` itself when it fits in [`MESSAGE_MAX_BYTES`], else its head, the
/// [`MESSAGE_CUT_MARKER`] and its tail, cut on character boundaries: at most
/// `MESSAGE_MAX_BYTES` and at most 3 bytes short of it.
pub fn clamp(text: &str) -> String {
    clamp_with(text, MESSAGE_MAX_BYTES, MESSAGE_CUT_MARKER)
}

/// The queued messages of one delivery, in queue order, separated by one blank line,
/// then clamped: decision 29's one new turn.
pub fn join_turn(messages: &[&Outgoing]) -> String {
    let joined = messages
        .iter()
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    clamp(&joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outgoing(id: u64, text: &str) -> Outgoing {
        Outgoing {
            id,
            window_id: 4,
            task_id: "t1".into(),
            text: text.into(),
            queued_at: 100 + id,
            delivered_at: None,
        }
    }

    /// `世` is 3 bytes and `MESSAGE_MAX_BYTES` (32 768) is not a multiple of 3, so the
    /// cuts must back off to a character boundary; the bound is asserted from both
    /// sides, so a clamp that dropped half its budget would fail too.
    #[test]
    fn clamp_keeps_head_and_tail_on_char_boundaries() {
        assert_eq!(MESSAGE_MAX_BYTES % 3, 2);
        for prefix in ["", "a", "ab"] {
            let text = format!("{prefix}{}", "世".repeat((40_002 - prefix.len()) / 3));
            let out = clamp(&text);
            assert!(out.len() <= MESSAGE_MAX_BYTES, "{prefix:?}: {}", out.len());
            assert!(
                out.len() >= MESSAGE_MAX_BYTES - 3,
                "{prefix:?}: {}",
                out.len()
            );
            let (head, tail) = out
                .split_once(MESSAGE_CUT_MARKER)
                .expect("the clamp marks its cut");
            assert!(text.starts_with(head) && !head.is_empty(), "{prefix:?}");
            assert!(text.ends_with(tail) && !tail.is_empty(), "{prefix:?}");
            assert!(head.len() + tail.len() < text.len());
        }
        let short = "世".repeat(100);
        assert_eq!(clamp(&short), short);
        let exact = "a".repeat(MESSAGE_MAX_BYTES);
        assert_eq!(clamp(&exact), exact);
    }

    #[test]
    fn join_turn_orders_and_separates() {
        let (a, b, c) = (
            outgoing(3, "[anthrex] first"),
            outgoing(4, "[anthrex] second\nwith two lines"),
            outgoing(5, "[anthrex] third"),
        );
        assert_eq!(
            join_turn(&[&a, &b, &c]),
            "[anthrex] first\n\n[anthrex] second\nwith two lines\n\n[anthrex] third"
        );
        assert_eq!(join_turn(&[&b]), b.text);
        let big = outgoing(6, &"x".repeat(MESSAGE_MAX_BYTES));
        let joined = join_turn(&[&a, &big]);
        assert!(joined.len() <= MESSAGE_MAX_BYTES);
        assert!(joined.starts_with("[anthrex] first\n\nxxx"));
    }
}
