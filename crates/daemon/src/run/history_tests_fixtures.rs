//! Builders for `history_tests.rs` (split out to keep it under the 600-line rule).

use proto::{
    AgentRole, Effort, Finding, Route, RoutingDecision, Runtime, Severity, Strength, TokenUsage,
    Verdict,
};

use crate::run::model::{AgentRound, CheckRecord, ReviewRecord, Run};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

pub(super) const STANDARD: &str =
    "[task.route]\nruntime = \"claude\"\nstrength = \"standard\"\neffort = \"medium\"";

pub(super) fn run_of(tasks: &[&str]) -> Run {
    let tasks: Vec<String> = tasks
        .iter()
        .map(|id| task_toml(id, "S", &format!("[\"crates/{id}/**\"]"), STANDARD))
        .collect();
    let mut run = run_ok(&plan_with(PROFILE, &tasks));
    run.repo_dir = "/tmp/data/repos/x".into();
    run
}

pub(super) fn route(runtime: Runtime, model: &str, strength: Strength, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.into(),
        strength,
        effort,
    }
}

pub(super) fn usage(n: u64) -> TokenUsage {
    TokenUsage {
        input: n,
        output: n + 1,
        cache_read: n + 2,
        cache_write: n + 3,
    }
}

/// A finished round of `role`, its counters from the caller.
pub(super) fn round(
    role: AgentRole,
    session: u32,
    tool_calls: u32,
    used: TokenUsage,
) -> AgentRound {
    let json = serde_json::json!({
        "role": role, "session": session, "round": session, "window_id": 7,
        "route": route(Runtime::Claude, "claude-sonnet-5", Strength::Standard, Effort::High),
        "launch_op": 1, "session_id": "s", "pid": null, "ended": true, "started_at": 1_000,
        "ended_at": 1_500, "turn_open": false, "turns": 1, "turn_had_task_done": true,
        "last_event": 1_500, "tool_calls": tool_calls, "rate_limited_until": null,
        "in_retry_streak": false, "open_subagents": [], "denials": 0, "usage": used,
        "deaths": 0, "fallback": "None", "stall": "Watching", "failed_turn": "None",
        "review_nudged": false, "wrap_up_sent": false, "retiring": false,
        "delivery_failures": 0, "turn_denied": [],
    });
    serde_json::from_value(json).expect("a round")
}

pub(super) fn check(ok: bool, on_candidate: bool) -> CheckRecord {
    CheckRecord {
        at: 1_200,
        ok,
        code: Some(if ok { 0 } else { 1 }),
        timed_out: false,
        tail: String::new(),
        secs: 4,
        on_candidate,
        summary: None,
        summary_source: None,
    }
}

pub(super) fn finding(severity: Severity) -> Finding {
    Finding {
        severity,
        file: None,
        line: None,
        input: None,
        text: "x".into(),
    }
}

pub(super) fn review(round: u32, verdict: Verdict, findings: Vec<Finding>) -> ReviewRecord {
    ReviewRecord {
        round,
        route: route(Runtime::Codex, "", Strength::Standard, Effort::Low),
        base: "b".into(),
        head: "h".into(),
        verdict: Some(verdict),
        summary: String::new(),
        findings,
    }
}

/// A decision's invariant: its selected candidate is its chosen route, unskipped.
pub(super) fn assert_selected(d: &RoutingDecision) {
    let chosen = &d.candidates[d.selected_index as usize];
    assert_eq!(chosen.route, d.chosen, "{d:#?}");
    assert_eq!(chosen.skipped_reason, None, "{d:#?}");
}

pub(super) fn candidates(d: &RoutingDecision) -> Vec<(String, Effort, Option<String>)> {
    d.candidates
        .iter()
        .map(|c| {
            let name = format!("{}:{}", c.route.runtime, c.route.model);
            (name, c.route.effort, c.skipped_reason.clone())
        })
        .collect()
}

pub(super) fn s(text: &str) -> Option<String> {
    Some(text.to_string())
}
