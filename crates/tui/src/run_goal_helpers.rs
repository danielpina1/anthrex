//! The goal form's small helpers (cleaning a pasted line, the choices' cycles), moved
//! out of `run_goal.rs` unchanged per the 600-line rule (milestone 9.6 task 18).

use crate::dialog::TextInput;
use crate::run_edit::TEXT_MAX_CHARS;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DeliveryMode, Runtime};

/// A tab becomes a space, and every control or hidden format character is dropped (a
/// pasted line break too: the custom model is one line).
pub(super) fn clean_line(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if c.is_control() || crate::safe_text::is_hidden_format(c) => None,
            c => Some(c),
        })
        .collect()
}

pub(super) fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
}

pub(super) fn insert_bounded(input: &mut TextInput, text: &str) {
    let room = TEXT_MAX_CHARS.saturating_sub(input.text().chars().count());
    if room == 0 || text.is_empty() {
        return;
    }
    let cut: String = text.chars().take(room).collect();
    input.insert(&cut);
}

/// `configured`, `local`, `pr`, round.
pub(super) fn next_delivery(value: Option<DeliveryMode>, forward: bool) -> Option<DeliveryMode> {
    let order = [None, Some(DeliveryMode::Local), Some(DeliveryMode::Pr)];
    cycle(&order, value, forward)
}

/// Milestone 9.6: `configured`, `full`, `off`, round.
pub(super) fn next_design(
    value: Option<proto::DesignMode>,
    forward: bool,
) -> Option<proto::DesignMode> {
    let order = [
        None,
        Some(proto::DesignMode::Full),
        Some(proto::DesignMode::Off),
    ];
    cycle(&order, value, forward)
}

/// The value after (or before) `value` in `order`, round; the first when not listed.
pub(crate) fn cycle<T: Copy + PartialEq>(order: &[T], value: T, forward: bool) -> T {
    let at = order.iter().position(|v| *v == value).unwrap_or(0);
    let len = order.len();
    order[if forward {
        (at + 1) % len
    } else {
        (at + len - 1) % len
    }]
}

pub(super) fn next_runtime(value: Option<Runtime>, forward: bool) -> Option<Runtime> {
    let order = [None, Some(Runtime::Claude), Some(Runtime::Codex)];
    cycle(&order, value, forward)
}
