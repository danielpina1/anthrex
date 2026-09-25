//! M8a.12: denials (decision 32, with fix round 1's tool names), and fix round 1's
//! minor findings m-1, m-3 and m-5 to m-7 (ruling T12-minors).

use proto::{AgentRole, BlockReason, TaskState, TokenUsage};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::holds::delivers;
use super::turns::working_on;
use crate::headless::FailureKind;
use crate::run::contract::{DENIAL_LISTED_REASON, denied_text, rate_limit_continue};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpResult, TurnOutcome};

const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";

fn args() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

fn working() -> (Fixture, u32) {
    working_on("")
}

fn block_of(fx: &Fixture) -> (BlockReason, String) {
    let block = fx.task("t1").block.clone().expect("t1 is blocked");
    (block.reason, block.text)
}

#[test]
fn denials_block_at_the_threshold() {
    let deny = AgentSignal::PermissionDenied {
        tool: "Write".into(),
        reason: "not allowed".into(),
    };
    let ended = |tools: &[&str]| AgentSignal::TurnEnded {
        outcome: TurnOutcome::Completed,
        usage: None,
        denials: tools.iter().map(|t| t.to_string()).collect(),
    };
    // Two seen as events, and the result lists them plus one more.
    let (mut fx, window) = working();
    fx.signal(window, deny.clone());
    fx.signal(window, deny.clone());
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let effects = fx.signal(window, ended(&["Bash", "Write", "Write"]));
    assert_eq!(
        block_of(&fx),
        (
            BlockReason::Environment,
            denied_text(3, "Bash", DENIAL_LISTED_REASON)
        ),
        "the last denial is the one only the result listed"
    );
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    assert!(ops_in(&effects, "CountCommits").is_empty());

    // A denial seen as an event is not counted again from `permission_denials`.
    let (mut fx, window) = working();
    fx.signal(window, deny.clone());
    fx.signal(window, deny.clone());
    fx.signal(window, ended(&["Write", "Write"]));
    assert_eq!(fx.task("t1").rounds[0].denials, 2);
    assert_eq!(fx.task("t1").state, TaskState::Working);

    // Review m-4 (probe G): denials listed only in the result name their tool.
    let (mut fx, window) = working();
    fx.signal(window, ended(&["Write", "Write", "Write"]));
    assert_eq!(
        block_of(&fx),
        (
            BlockReason::Environment,
            denied_text(3, "Write", DENIAL_LISTED_REASON)
        )
    );

    // An event's denial is still the last one when the result lists nothing new.
    let (mut fx, window) = working();
    fx.signal(window, ended(&["Bash"]));
    fx.signal(window, AgentSignal::TurnStarted);
    fx.signal(window, deny.clone());
    fx.signal(window, deny);
    assert_eq!(
        block_of(&fx),
        (
            BlockReason::Environment,
            denied_text(3, "Write", "not allowed")
        )
    );
}

/// Review m-3: a completed turn resets the two-`Other` rule.
#[test]
fn a_completed_turn_resets_the_two_other_failures_rule() {
    let (mut fx, window) = working_on(ROOMY);
    let other = |e: &str| TurnOutcome::Failed {
        error: e.into(),
        kind: FailureKind::Other,
    };
    fx.turn_ended(window, other("overloaded"));
    let at = fx.now;
    fx.send(at + 301, EventKind::Tick);
    fx.turn_completed(window);
    fx.signal(window, AgentSignal::TurnStarted);
    fx.turn_ended(window, other("overloaded later"));
    assert_eq!(fx.task("t1").state, TaskState::Working, "not in a row");
    let at = fx.now;
    let effects = fx.send(at + 301, EventKind::Tick);
    let continued = delivers(&effects)
        .iter()
        .any(|t| t.contains(&rate_limit_continue("overloaded later")));
    assert!(continued, "{effects:#?}");
}

/// Review m-1: decision 32's rejections come before the protected and spill checks.
#[test]
fn rejections_come_before_the_spill_split() {
    for edit_result in [
        (|r: &mut OpResult| {
            if let OpResult::DoneChecked { dirty_tracked, .. } = r {
                *dirty_tracked = 1;
            }
        }) as fn(&mut OpResult),
        |r: &mut OpResult| {
            if let OpResult::DoneChecked { head_branch, .. } = r {
                *head_branch = None;
            }
        },
    ] {
        let (mut fx, window) = working_on(ROOMY);
        let effects = fx.tool(window, "task_done", args());
        let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
        let mut result = fx.clean_check("t1");
        edit_result(&mut result);
        if let OpResult::DoneChecked {
            outside_owns,
            protected_changed,
            ..
        } = &mut result
        {
            *outside_owns = vec!["x.rs".into()];
            *protected_changed = vec!["AGENTS.md".into()];
        }
        let effects = fx.done(op, result);
        let reply = replies(&effects);
        assert!(
            reply[0]
                .as_ref()
                .unwrap_err()
                .starts_with("task_done rejected: "),
            "{reply:?}"
        );
        let t1 = fx.task("t1");
        assert_eq!((t1.state, t1.failures), (TaskState::Working, 0));
    }
}

/// Review m-1: a window being killed is not the current worker.
#[test]
fn a_retiring_window_cannot_claim() {
    let (mut fx, window) = working_on(ROOMY);
    fx.task_mut("t1").rounds[0].retiring = true;
    let effects = fx.tool(window, "task_done", args());
    assert_eq!(
        replies(&effects),
        vec![Err(
            "this window is not the current worker of task t1".to_string()
        )]
    );
    assert!(ops_in(&effects, "VerifyDone").is_empty());
}

/// Ruling T12-m5: a reviewer's spend stays on its own round; the task's total (and
/// rung 4) counts worker rounds only.
#[test]
fn reviewer_spend_is_not_the_tasks() {
    let plan = plan_with(
        &PROFILE.replace("check = \"cargo test\"\n", ""),
        &[task_toml(
            "t1",
            "S",
            "[\"docs/**\"]",
            "test_mode = \"none\"\ntest_mode_reason = \"docs\"",
        )],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    fx.launch_all();
    let effects = fx.force("t1", TaskState::Review);
    let (op, _) = ops_in(&effects, "PrepareReview")[0].clone();
    fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: HEAD.into(),
            patch: String::new(),
        },
    );
    let reviewer = fx.complete_windows()[0].1;
    fx.signal(
        reviewer,
        AgentSignal::ToolUse {
            name: "Read".into(),
        },
    );
    fx.signal(
        reviewer,
        AgentSignal::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: Some(TokenUsage {
                input: 10,
                output: 10,
                cache_read: 0,
                cache_write: 0,
            }),
            denials: Vec::new(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.spent_total.tool_calls, 0);
    assert_eq!(t1.spent_total.tokens, 0);
    let r = t1.rounds.last().unwrap();
    assert_eq!(
        (r.role, r.tool_calls, r.usage.billable()),
        (AgentRole::Reviewer, 1, 20)
    );
}

/// Review m-6: a huge token budget does not overflow the hard-limit check.
#[test]
fn a_huge_token_budget_does_not_overflow() {
    let extra = "[task.budget]\ntool_calls = 1000\nminutes = 1000\ntokens = 9223372036854775807";
    let (mut fx, window) = working_on(extra);
    let effects = fx.signal(
        window,
        AgentSignal::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: Some(TokenUsage {
                input: u64::MAX / 2,
                output: 0,
                cache_read: 0,
                cache_write: 0,
            }),
            denials: Vec::new(),
        },
    );
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
}

/// Review m-7: the reply to a bounce at rung 2 says the session is being replaced.
#[test]
fn a_rung_2_bounce_reply_says_the_session_is_replaced() {
    let profile = PROFILE.replace("setup = ", "generated = [\"Cargo.lock\"]\nsetup = ");
    let mut fx = Fixture::new(&plan_with(&profile, &[task("t1", "S", "a", ROOMY)]));
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    let mut last = Vec::new();
    for _ in 0..2 {
        let effects = fx.tool(window, "task_done", args());
        let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
        let mut result = fx.clean_check("t1");
        if let OpResult::DoneChecked {
            generated_outside_owns,
            ..
        } = &mut result
        {
            *generated_outside_owns = vec!["Cargo.lock".into()];
        }
        last = fx.done(op, result);
    }
    let reply = replies(&last)[0].clone().unwrap_err();
    assert_eq!(fx.task("t1").rung, 2);
    assert!(reply.contains("being replaced"), "{reply}");
    assert!(!reply.contains("call task_done again"), "{reply}");
}
