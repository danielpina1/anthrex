//! M8a.13: severity-aware review by a fresh, read-only reviewer session each round
//! (decision 35), its failures on decision 38's ladder, and `run override`.

use proto::{AgentRole, BlockReason, Finding, Runtime, Severity, TaskState, Verdict};
use serde_json::json;

use super::dispatch::{replies, task_path};
use super::fixture::*;
use super::gates::{CHECK_MODE, accepted, check_result, only_op, working_on, working_with};
use super::holds::delivers;
use super::turns_fixes::assert_alive;
use crate::run::contract::{
    APPROVE_WITH_BLOCKING, REVIEW_DIFF_MAX, REVIEW_NUDGE, REVIEW_RECORDED, REVIEWER_STOPPED_TWICE,
    review_changes_message,
};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};
use crate::run::model::{OpId, ReviewLevel};
use crate::run::roster::pick_reviewer;

pub(super) const CODEX_AUTHOR: &str = "[task.route]\nruntime = \"codex\"\nmodel = \"\"";

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
    let (mut fx, window) = working_on(profile, &format!("{CHECK_MODE}\n{extra}"));
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

#[test]
fn review_round_uses_a_fresh_session_and_worktree() {
    for (extra, reviewer_runtime) in [("", Runtime::Codex), (CODEX_AUTHOR, Runtime::Claude)] {
        let (mut fx, window) = working_on(PROFILE, &format!("{CHECK_MODE}\n{extra}"));
        let (op, kind) = in_review(&mut fx, window);
        let t1 = fx.task("t1");
        assert_eq!(
            kind,
            OpKind::PrepareReview {
                root: "/tmp/x".into(),
                head_ref: HEAD.into(),
                base_ref: BASE.into(),
                path: task_path("t1.review"),
            }
        );
        assert_eq!(t1.gate_op, Some(op));
        let level = t1.review_level.unwrap();
        let route = pick_reviewer(&fx.run().roster, &t1.route, level);
        let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
        let OpKind::CreateWindow {
            name,
            spec,
            worktree,
            ..
        } = kind
        else {
            unreachable!()
        };
        assert_eq!(name, format!("{H4}/t1.r1"));
        assert_eq!(worktree, task_path("t1.review"));
        assert_eq!(spec.runtime, reviewer_runtime);
        assert_eq!(
            (spec.runtime, spec.model.clone()),
            (route.runtime, route.model.clone())
        );
        let mcp = spec.mcp.clone().unwrap();
        assert_eq!(
            (mcp.role, mcp.task_id.as_deref()),
            (AgentRole::Reviewer, Some("t1"))
        );
        assert_eq!(
            spec.allowed_tools,
            [
                "mcp__anthrex__submit_review",
                "Read",
                "Glob",
                "Grep",
                "Bash(git diff:*)",
                "Bash(git log:*)",
                "Bash(git show:*)"
            ]
        );
        // M8a.7 moved reviewers from `plan` to `dontAsk` (plan mode blocks the MCP call
        // in `-p`), with the write tools disallowed.
        if reviewer_runtime == Runtime::Claude {
            assert_eq!(spec.claude_permission_mode.as_deref(), Some("dontAsk"));
            assert_eq!(
                spec.claude_disallowed_tools,
                ["Edit", "Write", "NotebookEdit"]
            );
            // F1c round 3 (N4): a Claude reviewer runs under a read-only sandbox.
            let sandbox = spec.claude_sandbox.as_ref().expect("read-only sandbox");
            assert!(sandbox.writable_roots.is_empty());
        } else {
            assert_eq!(spec.codex_sandbox, "read-only");
            assert!(spec.codex_writable_roots.is_empty());
            assert_eq!(spec.claude_sandbox, None);
        }
        let review = &fx.task("t1").reviews[0];
        assert_eq!((review.round, review.verdict), (1, None));
        assert_alive(&fx);
    }

    // Round 2 after a rejection: `t1.r2`, and a prompt listing round 1's blocking
    // findings to confirm fixed.
    let (mut fx, window, rwindow) = reviewed(PROFILE, "");
    submit(&mut fx, rwindow, verdict("changes", blocking()));
    // The retired reviewer's process ends (T13-I2: round 2 waits for it).
    super::turns::exited(&mut fx, rwindow);
    fx.turn_completed(window);
    let (op, _) = in_review(&mut fx, window);
    let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let OpKind::CreateWindow {
        name, first_turn, ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(name, format!("{H4}/t1.r2"));
    let listed = first_turn
        .split_once("Earlier findings to confirm fixed:\n")
        .expect("round 1's findings")
        .1;
    assert!(listed.starts_with(
        "- [critical] crates/a/src/x.rs:3 critical text\n- [important] input an empty name: important text\n"
    ));
    assert!(!first_turn.contains("minor text"));
}

#[test]
fn approve_and_minor_only_changes_both_go_to_the_merge_queue() {
    let minor = vec![finding(
        "minor",
        json!({"file": "crates/a/src/y.rs", "line": 9}),
    )];
    for kind in ["approve", "changes"] {
        let (mut fx, _, rwindow) = reviewed(PROFILE, "");
        let effects = submit(&mut fx, rwindow, verdict(kind, minor.clone()));
        assert_eq!(replies(&effects), vec![Ok(REVIEW_RECORDED.to_string())]);
        assert!(effects.contains(&Effect::RetireWindow { window_id: rwindow }));
        let t1 = fx.task("t1");
        assert_eq!(t1.state, TaskState::MergeQueue, "{kind}");
        assert_eq!(fx.run().merge_queue, vec!["t1".to_string()]);
        assert_eq!((t1.failures, t1.bounces.review), (0, 0));
        let review = &t1.reviews[0];
        let expected = if kind == "approve" {
            Verdict::Approve
        } else {
            Verdict::Changes
        };
        assert_eq!(review.verdict, Some(expected));
        assert_eq!(
            review.findings,
            vec![Finding {
                severity: Severity::Minor,
                file: Some("crates/a/src/y.rs".into()),
                line: Some(9),
                input: None,
                text: "minor text".into(),
            }]
        );
        assert_eq!(review.summary, "looked");
    }
}

#[test]
fn a_blocking_finding_is_a_review_failure() {
    let (mut fx, window, rwindow) = reviewed(PROFILE, "");
    fx.turn_completed(window);
    let effects = submit(&mut fx, rwindow, verdict("changes", blocking()));
    assert_eq!(replies(&effects), vec![Ok(REVIEW_RECORDED.to_string())]);
    assert!(effects.contains(&Effect::RetireWindow { window_id: rwindow }));
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.rung, t1.failures, t1.bounces.review),
        (TaskState::Working, 1, 1, 1)
    );
    let text = review_changes_message(&t1.reviews[0]);
    assert_eq!(
        text,
        "[anthrex] Review round 1 asked for changes. Fix every finding below, commit, then call task_done again.\n- [critical] crates/a/src/x.rs:3 critical text\n- [important] input an empty name: important text"
    );
    let delivered: Vec<(u32, String)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver {
                window_id, text, ..
            } => Some((*window_id, text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(delivered, vec![(window, text)]);
    assert!(fx.run().merge_queue.is_empty());
    assert_alive(&fx);
}

#[test]
fn finding_validation() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    let effects = submit(
        &mut fx,
        rwindow,
        verdict(
            "changes",
            vec![finding("critical", json!({"file": "a.rs"}))],
        ),
    );
    assert_eq!(
        replies(&effects),
        vec![Err(
            "invalid arguments: findings[0]: a critical or important finding needs file and line, or input".to_string()
        )]
    );
    let effects = submit(
        &mut fx,
        rwindow,
        verdict("approve", vec![finding("important", json!({"input": "x"}))]),
    );
    assert_eq!(
        replies(&effects),
        vec![Err(APPROVE_WITH_BLOCKING.to_string())]
    );
    for bad in [
        json!({"verdict": "maybe", "summary": "s", "findings": []}),
        json!({"verdict": "approve", "findings": []}),
        json!({"verdict": "approve", "summary": "s"}),
        json!({"verdict": "approve", "summary": "s", "findings": [], "extra": 1}),
        verdict("approve", vec![json!({"severity": "minor"})]),
        verdict("approve", vec![finding("minor", json!({"line": 0}))]),
    ] {
        let effects = submit(&mut fx, rwindow, bad.clone());
        let reply = replies(&effects);
        assert!(
            matches!(&reply[..], [Err(e)] if e.starts_with("invalid arguments: ")),
            "{bad}: {reply:?}"
        );
    }
    // Nothing was recorded; the reviewer can still submit a valid verdict.
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert_eq!(fx.task("t1").reviews[0].verdict, None);
    let effects = submit(&mut fx, rwindow, verdict("approve", vec![]));
    assert_eq!(replies(&effects), vec![Ok(REVIEW_RECORDED.to_string())]);
}

#[test]
fn a_second_submit_in_the_same_round_is_refused() {
    let (mut fx, window, rwindow) = reviewed(PROFILE, "");
    submit(&mut fx, rwindow, verdict("approve", vec![]));
    let effects = submit(&mut fx, rwindow, verdict("changes", blocking()));
    assert_eq!(
        replies(&effects),
        vec![Err("a review for round 1 was already submitted".to_string())]
    );
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    // Other callers are refused too.
    let effects = fx.tool_as(
        AgentRole::Worker,
        window,
        "t1",
        "submit_review",
        verdict("approve", vec![]),
    );
    assert_eq!(
        replies(&effects),
        vec![Err("this window is not the reviewer of task t1".to_string())]
    );
}

#[test]
fn a_reviewer_turn_without_a_verdict_is_nudged_then_replaced() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    let level = fx.task("t1").review_level;
    let effects = fx.turn_completed(rwindow);
    let nudges: Vec<(u32, String)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver {
                window_id, text, ..
            } => Some((*window_id, text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(nudges, vec![(rwindow, REVIEW_NUDGE.to_string())]);
    let ids: Vec<u64> = fx.run().outbox.iter().map(|m| m.id).collect();
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: true,
        error: None,
    });
    assert_alive(&fx);

    // The nudge's turn also ends without a verdict: round 2, same level, no failure.
    let effects = fx.turn_completed(rwindow);
    // A Codex reviewer between turns has no process: its round ends at once (T13-I2).
    let last = fx.task("t1").rounds.last().unwrap().clone();
    assert!(last.retiring && last.ended, "{last:#?}");
    only_op(&effects, "PrepareReview");
    assert_alive(&fx);
    let (op, _) = fx.op("PrepareReview");
    let (rwindow2, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let OpKind::CreateWindow { name, .. } = kind else {
        unreachable!()
    };
    assert_eq!(name, format!("{H4}/t1.r2"));
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.review_level, t1.failures),
        (TaskState::Review, level, 0)
    );
    assert_alive(&fx);

    // A second verdict-less round in a row blocks the task.
    fx.turn_completed(rwindow2);
    fx.turn_completed(rwindow2);
    let last = fx.task("t1").rounds.last().unwrap().clone();
    assert!(last.retiring && last.ended, "{last:#?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.failures, 0);
    let block = t1.block.clone().unwrap();
    assert_eq!(
        (block.reason, block.text.as_str()),
        (BlockReason::Environment, REVIEWER_STOPPED_TWICE)
    );
    assert!(ops_in(&fx.tick(), "PrepareReview").is_empty());
}

/// Decision 35's resume rule for a reviewer: its process dying mid-turn is resumed
/// once; the second death in the round ends the round without a verdict.
#[test]
fn a_reviewer_that_dies_twice_ends_its_round() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.signal(
        rwindow,
        AgentSignal::Init {
            session_id: "r-1".into(),
        },
    );
    let effects = super::turns::exited(&mut fx, rwindow);
    let (op, kind) = only_op(&effects, "ResumeSession");
    let OpKind::ResumeSession {
        window_id,
        session_id,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!((window_id, session_id.as_str()), (rwindow, "r-1"));
    fx.done(op, OpResult::Resumed);
    assert_alive(&fx);
    let effects = super::turns::exited(&mut fx, rwindow);
    assert!(ops_in(&effects, "ResumeSession").is_empty());
    let (_, kind) = only_op(&effects, "PrepareReview");
    let OpKind::PrepareReview { .. } = kind else {
        unreachable!()
    };
    assert_eq!(fx.task("t1").review_misses, 1);
    assert_alive(&fx);
}

/// A reviewer turn silent for `stall_after_secs` ends its round without a verdict.
#[test]
fn a_silent_reviewer_is_replaced() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    let quiet = fx.task("t1").rounds.last().unwrap().last_event;
    // Ruling T24-clock: at least 600 real seconds, one engine second past them.
    let effects = fx.send(quiet + 600, EventKind::Tick);
    assert!(!effects.contains(&Effect::KillWindow { window_id: rwindow }));
    let effects = fx.send(quiet + 601, EventKind::Tick);
    assert!(
        effects.contains(&Effect::KillWindow { window_id: rwindow }),
        "{effects:#?}"
    );
    assert_eq!(fx.task("t1").review_misses, 1);
    assert_alive(&fx);
}

#[test]
fn review_small_off_skips_s_review() {
    let config = config::Orchestrator {
        review_small: false,
        ..Default::default()
    };
    // An S tdd task with a check: level small, skipped.
    let (mut fx, window) = working_with(PROFILE, "", config);
    assert_eq!(fx.task("t1").review_level, None);
    let effects = accepted(&mut fx, window, super::gates::tdd_args());
    let (op, _) = only_op(&effects, "Proof");
    let effects = fx.done(
        op,
        OpResult::Proof {
            red_failed: true,
            head_passed: true,
            matched: true,
            red_tail: String::new(),
            head_tail: String::new(),
        },
    );
    let (op, _) = only_op(&effects, "Check");
    fx.done(op, check_result(true));
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    assert!(ops_in(&fx.log, "PrepareReview").is_empty());
}

#[test]
fn override_skips_review_and_marks_the_task() {
    let (mut fx, window, rwindow) = reviewed(PROFILE, "");
    // Not while working.
    let (mut other, _) = working_on(PROFILE, CHECK_MODE);
    let reply = other.reply();
    let effects = other.next(EventKind::Override {
        reply,
        run_id: RUN_ID.into(),
        task_id: "t1".into(),
        reason: "trust me".into(),
    });
    assert!(matches!(&replies(&effects)[..], [Err(_)]), "{effects:#?}");

    let reply = fx.reply();
    let effects = fx.next(EventKind::Override {
        reply,
        run_id: RUN_ID.into(),
        task_id: "t1".into(),
        reason: "urgent fix".into(),
    });
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert!(effects.contains(&Effect::KillWindow { window_id: rwindow }));
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::MergeQueue);
    assert_eq!(t1.merged_without_approval.as_deref(), Some("urgent fix"));
    assert_eq!(fx.run().merge_queue, vec!["t1".to_string()]);
    // The reviewer's late verdict no longer applies.
    let effects = submit(&mut fx, rwindow, verdict("changes", blocking()));
    assert!(matches!(&replies(&effects)[..], [Err(_)]));
    assert_eq!(fx.task("t1").failures, 0);
}

#[test]
fn the_reviewer_prompt_carries_the_clamped_diff() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let (op, _) = in_review(&mut fx, window);
    let patch: String = (0..200u32)
        .map(|i| char::from(b'a' + (i * 7 % 26) as u8))
        .collect();
    let (_, kind) = reviewer(&mut fx, op, &patch);
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    let header = format!("Diff ({}..{}):\n", &BASE[..7], &HEAD[..7]);
    assert!(
        first_turn.contains(&format!("{header}{patch}\n")),
        "{first_turn}"
    );
    assert!(!first_turn.contains("[diff clamped"));

    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let (op, _) = in_review(&mut fx, window);
    let patch = format!("ab{}", "世".repeat(13_333));
    assert_eq!(patch.len(), 40_001);
    let (_, kind) = reviewer(&mut fx, op, &patch);
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    let after = first_turn.split_once(&header).expect("the diff block").1;
    let (block, rest) = after
        .split_once("\n[diff clamped: ")
        .expect("the clamp line");
    assert!(block.len() <= REVIEW_DIFF_MAX, "{}", block.len());
    assert!(block.len() >= REVIEW_DIFF_MAX - 3, "{}", block.len());
    let marker = crate::run::contract::DIFF_CUT_MARKER;
    let (head, tail) = block.split_once(marker).expect("head and tail");
    assert!(patch.starts_with(head) && patch.ends_with(tail));
    assert!(rest.contains(" bytes omitted; read the rest with git diff "));
}

#[test]
fn the_reviewer_prompt_never_names_the_author() {
    let roster = working_on(PROFILE, CHECK_MODE).0.run().roster.clone();
    assert_eq!(roster.len(), 4, "the built-in roster");
    for entry in roster {
        let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
        fx.task_mut("t1").route = proto::Route {
            runtime: entry.runtime,
            model: entry.model.clone(),
            strength: entry.strength,
            effort: proto::Effort::Medium,
        };
        let (op, _) = in_review(&mut fx, window);
        let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
        let OpKind::CreateWindow { first_turn, .. } = kind else {
            unreachable!()
        };
        let label = serde_json::to_value(entry.runtime).unwrap();
        let label = label.as_str().unwrap();
        assert!(
            !first_turn.to_lowercase().contains(label),
            "{label}: {first_turn}"
        );
        if !entry.model.is_empty() {
            assert!(
                !first_turn.contains(&entry.model),
                "{}: {first_turn}",
                entry.model
            );
        }
    }
}

/// The review level is kept across rounds, and the reviewer is picked against the
/// author's current route: a rung-2 author on the peer runtime gets a reviewer on the
/// other one.
#[test]
fn a_review_after_rung_2_is_picked_against_the_new_author() {
    let (mut fx, window, rwindow) = reviewed(PROFILE, "");
    let _ = (window, rwindow);
    let level = fx.task("t1").review_level.unwrap_or(ReviewLevel::Medium);
    fx.task_mut("t1").route.runtime = Runtime::Codex;
    fx.task_mut("t1").route.model = String::new();
    fx.task_mut("t1").state = TaskState::Review;
    for round in fx.task_mut("t1").rounds.iter_mut() {
        if round.role == AgentRole::Reviewer {
            round.ended = true;
        }
    }
    let effects = fx.tick();
    let (op, _) = only_op(&effects, "PrepareReview");
    let (_, kind) = reviewer(&mut fx, op, "diff");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    let route = pick_reviewer(&fx.run().roster, &fx.task("t1").route, level);
    assert_eq!(spec.runtime, Runtime::Claude);
    assert_eq!(spec.model, route.model);
    let _ = delivers;
}
