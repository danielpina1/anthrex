//! The task edit form's choices: each field's next value forward or back (milestone 8c
//! decision 33).

use proto::{Effort, Runtime, Size, Strength, TestMode};

pub(super) fn next_runtime(value: Option<Runtime>, forward: bool) -> Option<Runtime> {
    let order = [None, Some(Runtime::Claude), Some(Runtime::Codex)];
    step(&order, value, forward)
}

pub(super) fn next_strength(value: Option<Strength>, forward: bool) -> Option<Strength> {
    let order = [
        None,
        Some(Strength::Fast),
        Some(Strength::Standard),
        Some(Strength::Frontier),
    ];
    step(&order, value, forward)
}

pub(super) fn next_effort(value: Option<Effort>, forward: bool) -> Option<Effort> {
    let order = [
        None,
        Some(Effort::Low),
        Some(Effort::Medium),
        Some(Effort::High),
    ];
    step(&order, value, forward)
}

pub(super) fn next_test_mode(value: TestMode, forward: bool) -> TestMode {
    step(
        &[TestMode::Tdd, TestMode::Check, TestMode::None],
        value,
        forward,
    )
}

/// `S ↔ M`; an `L` the plan opened with steps into them (`S` forward, `M` back).
pub(super) fn next_size(value: Size, forward: bool) -> Size {
    match (value, forward) {
        (Size::S, _) => Size::M,
        (Size::M, _) => Size::S,
        (Size::L, true) => Size::S,
        (Size::L, false) => Size::M,
    }
}

/// The neighbour of `value` in `order`, wrapping; a value not in `order` (a `shell`
/// runtime from a hand-written plan) steps to the first.
pub(super) fn step<T: Copy + PartialEq>(order: &[T], value: T, forward: bool) -> T {
    let len = order.len();
    match order.iter().position(|v| *v == value) {
        Some(at) if forward => order[(at + 1) % len],
        Some(at) => order[(at + len - 1) % len],
        None => order[0],
    }
}
