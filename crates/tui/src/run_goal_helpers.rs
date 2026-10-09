//! The goal form's small helpers (the choices' cycles), moved out of `run_goal.rs`
//! unchanged per the 600-line rule (milestone 9.6 task 18). Milestone 9.8: the runtime
//! cycle and the custom model's line went with them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::DeliveryMode;

pub(super) fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
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
