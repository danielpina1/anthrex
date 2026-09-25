//! M8a.12: budgets (decision 40), token metering, and the ladder's budget and rung-4
//! parts (decision 38).

use proto::{BlockReason, TaskState, TokenUsage};

use super::fixture::*;
use super::turns::{killed_exit, working_on};
use crate::run::contract::budget_wrap_up;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpResult, TurnOutcome};
use crate::run::snapshot::snapshot;

fn tool_use(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    fx.signal(
        window,
        AgentSignal::ToolUse {
            name: "Bash".into(),
        },
    )
}

fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> TokenUsage {
    TokenUsage {
        input,
        output,
        cache_read,
        cache_write,
    }
}

fn ended_with(fx: &mut Fixture, window: u32, usage: TokenUsage) -> Vec<Effect> {
    fx.signal(
        window,
        AgentSignal::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: Some(usage),
            denials: vec![],
        },
    )
}

fn kills(effects: &[Effect], window: u32) -> bool {
    effects.contains(&Effect::KillWindow { window_id: window })
}

/// Rung 2's fresh session, after the old one's exit and its diff: the new window.
fn fresh_session(fx: &mut Fixture, old: u32) -> u32 {
    let effects = killed_exit(fx, old);
    let (op, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let windows = fx.complete_windows();
    assert_eq!(windows.len(), 1);
    windows[0].1
}

#[test]
fn budget_soft_then_hard() {
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 5\nminutes = 1000");
    for _ in 0..4 {
        tool_use(&mut fx, window);
    }
    assert!(fx.run().outbox.is_empty());
    tool_use(&mut fx, window);
    let queued: Vec<String> = fx.run().outbox.iter().map(|m| m.text.clone()).collect();
    let budget = fx.task("t1").budget;
    let spent = proto::Spend {
        tool_calls: 5,
        secs: 0,
        tokens: 0,
    };
    assert_eq!(queued.len(), 1);
    assert!(queued[0].starts_with(&budget_wrap_up(spent, budget)[..60]));
    assert!(queued[0].contains("(5/5 tool calls"), "{}", queued[0]);
    tool_use(&mut fx, window);
    assert_eq!(fx.run().outbox.len(), 1, "once per session");
    let effects = tool_use(&mut fx, window);
    assert!(!kills(&effects, window), "7 is not above 7.5");
    let effects = tool_use(&mut fx, window);
    assert!(kills(&effects, window), "8 is above 7.5");
    let t1 = fx.task("t1");
    assert_eq!((t1.budget_exceeded, t1.rung, t1.failures), (1, 2, 0));

    let second = fresh_session(&mut fx, window);
    let info = &snapshot(&fx.state, fx.now).runs[0].tasks[0];
    assert_eq!(
        info.spent_session.tool_calls, 0,
        "the session spend restarts"
    );
    assert_eq!(info.spent_total.tool_calls, 8, "the total is kept");
    for _ in 0..7 {
        tool_use(&mut fx, second);
    }
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let effects = tool_use(&mut fx, second);
    assert!(kills(&effects, second));
    let t1 = fx.task("t1");
    assert_eq!((t1.budget_exceeded, t1.rung), (2, 3));
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::MisSized);

    // Minutes, through Tick.
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 1000\nminutes = 5");
    let started = fx.task("t1").rounds[0].started_at;
    let effects = fx.send(started + 299, EventKind::Tick);
    assert!(effects.is_empty(), "nothing due: {effects:#?}");
    fx.send(started + 300, EventKind::Tick);
    assert_eq!(fx.run().outbox.len(), 1, "the wrap-up at 5 minutes");
    let effects = fx.send(started + 420, EventKind::Tick);
    assert!(!kills(&effects, window), "7 minutes");
    let effects = fx.send(started + 449, EventKind::Tick);
    assert!(!kills(&effects, window), "7 minutes 29");
    let effects = fx.send(started + 450, EventKind::Tick);
    assert!(kills(&effects, window), "7.5 minutes is 1.5 × 5");
    // At 8 minutes, with no earlier tick, just the same.
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 1000\nminutes = 5");
    let started = fx.task("t1").rounds[0].started_at;
    fx.send(started + 420, EventKind::Tick);
    let effects = fx.send(started + 480, EventKind::Tick);
    assert!(kills(&effects, window), "8 minutes");
    assert_eq!(fx.task("t1").budget_exceeded, 1);

    // Tokens: billable is input + cache writes + output, never cache reads.
    let tokens = "[task.budget]\ntool_calls = 1000\nminutes = 1000\ntokens = 1001";
    let (mut fx, window) = working_on(tokens);
    let effects = ended_with(&mut fx, window, usage(700, 600, 5000, 300));
    assert!(kills(&effects, window), "1600 is above 1501.5");
    let (mut fx, window) = working_on(tokens);
    let effects = ended_with(&mut fx, window, usage(601, 600, 5000, 300));
    assert!(!kills(&effects, window), "1501 is not");
    assert_eq!(fx.task("t1").budget_exceeded, 0);
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 1000\nminutes = 1000");
    let effects = ended_with(&mut fx, window, usage(9_000_000, 9_000_000, 0, 9_000_000));
    assert!(!kills(&effects, window), "no token budget, no breach");
    assert!(fx.run().outbox.is_empty());
}

#[test]
fn usage_is_summed_per_round_and_task() {
    let (mut fx, window) = working_on("");
    ended_with(&mut fx, window, usage(11, 23, 37, 41));
    fx.signal(window, AgentSignal::TurnStarted);
    ended_with(&mut fx, window, usage(101, 203, 307, 409));
    let t1 = fx.task("t1");
    assert_eq!(t1.rounds[0].usage, usage(112, 226, 344, 450));
    assert_eq!(t1.spent_total.tokens, 788);
    let info = &snapshot(&fx.state, fx.now).runs[0].tasks[0];
    assert_eq!(info.rounds[0].usage, usage(112, 226, 344, 450));
    assert_eq!(info.spent_total.tokens, 788);
    assert_eq!(info.spent_session.tokens, 788);
}

#[test]
fn rung_4_blocks_on_the_next_size_ceiling() {
    let (mut fx, window) = working_on("");
    // The M budget is 150 tool calls (decision 40).
    assert_eq!(fx.run().limits.budget_m.tool_calls, 150);
    fx.task_mut("t1").spent_total.tool_calls = 148;
    let effects = tool_use(&mut fx, window);
    assert_eq!(fx.task("t1").state, TaskState::Working, "149: {effects:#?}");
    let effects = tool_use(&mut fx, window);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::Human);
    assert_eq!(t1.rung, 4);
    assert!(kills(&effects, window));

    // Minutes too, through Tick: M's 60 minutes.
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 1000\nminutes = 1000");
    let started = fx.task("t1").rounds[0].started_at;
    // Silence would stall first; keep the stream alive.
    for minute in 1..60 {
        fx.send(
            started + minute * 60,
            EventKind::Signal {
                window_id: window,
                signal: AgentSignal::Activity,
            },
        );
    }
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let effects = fx.send(started + 3600, EventKind::Tick);
    assert_eq!(
        fx.task("t1").block.as_ref().unwrap().reason,
        BlockReason::Human
    );
    assert!(kills(&effects, window));
}

#[test]
fn counter_changes_from_tool_calls_stay_lazy() {
    let (mut fx, window) = working_on("");
    let effects = tool_use(&mut fx, window);
    assert!(
        effects.contains(&Effect::Persist {
            run_id: RUN_ID.into(),
            urgent: false
        }),
        "{effects:#?}"
    );
    assert!(effects.contains(&Effect::Publish { structural: false }));
}
