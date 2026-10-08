//! The review fixtures `gates_review.rs` and its neighbours share (moved out of
//! `gates_review.rs` by task M9.8.9's fix round 1, for the 600-line rule).

use proto::{AgentRole, TaskState};
use serde_json::json;

use super::fixture::*;
use super::gates::{CHECK_MODE, accepted, check_result, only_op, working_with};
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::OpId;

/// A check-mode `t1` whose check passed: its `PrepareReview`.
pub(super) fn in_review(fx: &mut Fixture, window: u32) -> (OpId, OpKind) {
    let effects = accepted(fx, window, json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(true));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    only_op(&effects, "PrepareReview")
}

/// The reviewer session for a prepared review with `patch`: its window and launch op.
pub(super) fn reviewer(fx: &mut Fixture, op: OpId, patch: &str) -> (u32, OpKind) {
    let effects = fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: HEAD.into(),
            patch: patch.into(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::WatchWorktree { .. })),
        "a review worktree is never watched: {effects:#?}"
    );
    let window = fx.complete_windows()[0].1;
    (window, kind)
}

/// A working `t1` under review by a live reviewer: (fixture, worker, reviewer).
pub(super) fn reviewed(profile: &str, extra: &str) -> (Fixture, u32, u32) {
    reviewed_with(profile, extra, config::Orchestrator::default())
}

/// [`reviewed`] under `config` (milestone 9.8: [`codex_small`] for a Codex author, as a
/// plan's route is ignored, decision 31).
pub(super) fn reviewed_with(
    profile: &str,
    extra: &str,
    config: config::Orchestrator,
) -> (Fixture, u32, u32) {
    let (mut fx, window) = working_with(profile, &format!("{CHECK_MODE}\n{extra}"), config);
    let (op, _) = in_review(&mut fx, window);
    let (rwindow, _) = reviewer(&mut fx, op, "diff --git a/x b/x");
    (fx, window, rwindow)
}

pub(super) fn submit(fx: &mut Fixture, rwindow: u32, args: serde_json::Value) -> Vec<Effect> {
    fx.tool_as(AgentRole::Reviewer, rwindow, "t1", "submit_review", args)
}

pub(super) fn finding(severity: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut f = json!({"severity": severity, "text": format!("{severity} text")});
    f.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    f
}

pub(super) fn verdict(verdict: &str, findings: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"verdict": verdict, "summary": "looked", "findings": findings})
}

pub(super) fn blocking() -> Vec<serde_json::Value> {
    vec![
        finding("critical", json!({"file": "crates/a/src/x.rs", "line": 3})),
        finding("important", json!({"input": "an empty name"})),
        finding("minor", json!({"file": "crates/a/src/y.rs"})),
    ]
}
