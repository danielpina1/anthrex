//! Milestone 9.18 task 7, decision 9: rung 4's `own` is, per axis, the larger of the
//! class's budget and the task's `budget` (tokens: the larger when both are set, the
//! one that is set otherwise); `refit::ceiling` is unchanged.

use proto::Budget;

use super::fixture::*;
use crate::run::engine::ladder_budget::ceiling;
use crate::run::refit::{self, SizeClass};

/// A run of one S task `t1` with `extra` in its plan entry.
fn s_task(extra: &str) -> Fixture {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", extra)]));
    fx.ready(true);
    fx
}

fn rung4(fx: &Fixture) -> Budget {
    ceiling(fx.run(), fx.task("t1"))
}

#[test]
fn rung4_never_lowers_a_task_budget_above_its_class_ceiling() {
    let fx = s_task("[task.budget]\ntool_calls = 1000\nminutes = 1000");
    let m = fx.run().limits.budget_m;
    assert!(m.tool_calls < 1000 && m.minutes < 1000, "{m:?}");
    let next = rung4(&fx);
    assert_eq!((next.tool_calls, next.minutes), (1000, 1000), "{next:?}");
}

#[test]
fn rung4_with_a_task_budget_below_the_class_keeps_todays_ceiling() {
    let l = s_task("").run().limits.clone();
    let s = l.budget_s;
    let extra = format!(
        "[task.budget]\ntool_calls = {}\nminutes = {}",
        s.tool_calls, s.minutes
    );
    let fx = s_task(&extra);
    assert_eq!(fx.task("t1").budget, s);
    let next = rung4(&fx);
    assert_eq!(
        next,
        refit::ceiling(SizeClass::S, s, l.budget_m, l.budget_l)
    );
    // Today's value: the effective M budget (40/15 below 150/60).
    assert_eq!(
        next,
        Budget {
            tool_calls: 150,
            minutes: 60,
            tokens: None,
        }
    );
}

/// The S task's ceiling with M's tokens at 1,000,000, S's at `class`, and the task's
/// at `own`.
fn with_tokens(class: Option<u64>, own: Option<u64>) -> (Budget, Fixture) {
    let extra = match own {
        Some(t) => format!("[task.budget]\ntool_calls = 40\nminutes = 15\ntokens = {t}"),
        None => "[task.budget]\ntool_calls = 40\nminutes = 15".to_string(),
    };
    let mut fx = s_task(&extra);
    let limits = &mut fx.run_mut().limits;
    limits.budget_m.tokens = Some(1_000_000);
    limits.budget_s.tokens = class;
    (rung4(&fx), fx)
}

#[test]
fn the_token_axis_takes_the_task_tokens_when_the_class_has_none() {
    let (next, fx) = with_tokens(None, Some(5_000_000));
    let l = &fx.run().limits;
    let own = Budget {
        tokens: Some(5_000_000),
        ..l.budget_s
    };
    let expected = refit::ceiling(SizeClass::S, own, l.budget_m, l.budget_l);
    assert_eq!(expected.tokens, Some(5_000_000));
    assert_eq!(next, expected);
}

#[test]
fn the_token_axis_takes_the_larger_when_both_are_set() {
    let (next, _) = with_tokens(Some(3_000_000), Some(4_000_000));
    assert_eq!(next.tokens, Some(4_000_000), "the task's is larger");
    let (next, _) = with_tokens(Some(3_000_000), Some(2_000_000));
    assert_eq!(next.tokens, Some(3_000_000), "the class's is larger");
    let (next, _) = with_tokens(Some(3_000_000), None);
    assert_eq!(next.tokens, Some(3_000_000), "only the class's is set");
}
