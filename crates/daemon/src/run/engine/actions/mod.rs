//! Milestone 9.0.6 decisions 8 and 9: what a client may do on a run, a stage or a task.
//! `rules.rs` holds every run request's precondition, the one source of each refusal
//! text, which the request handlers call. Pure (design decision 2).

pub(crate) mod rules;

/// A node of a run's tree an action applies to.
// Task 7's `available` and `check` take it.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActionNode<'a> {
    Run,
    Stage(u16),
    Task(&'a str),
}
