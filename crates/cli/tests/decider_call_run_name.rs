//! The run title change: the `run_name` decider's call within its bound
//! (`decider::call::decide_within`), against real `fake-agent` processes scripted as
//! `run_name-<n>.json`. The fixtures are `support/decider.rs`; no test reaches a model.

mod support;

use std::time::{Duration, Instant};

use daemon::decider::call::decide_within;
use daemon::decider::{DeciderAnswer, DeciderRequest, RunNameInput};
use proto::{DeciderMode, DeciderSource};
use serde_json::json;
use support::decider::*;
use support::runtime;

/// Below the fixture context's own timeout (`TIMEOUT_SECS`), so the bound cuts the call.
const BOUND: Duration = Duration::from_secs(2);

fn request(goal: &str) -> DeciderRequest {
    DeciderRequest::RunName(RunNameInput { goal: goal.into() })
}

#[test]
fn a_scripted_answer_names_the_run() {
    let fx = Fixture::new();
    let answer = json!({"title": "Short run titles", "slug": "short-run-titles"});
    fx.script("run_name", 1, json!({ "answer": answer }));
    for mode in [DeciderMode::Claude, DeciderMode::Codex] {
        if mode == DeciderMode::Codex {
            fx.script("run_name", 2, json!({ "answer": answer }));
        }
        let goal = "in anthrex, at the bottom I want a short title";
        let (routed, decision) =
            runtime().block_on(decide_within(&fx.context(mode), &request(goal), BOUND));
        assert!(routed.is_some());
        assert_eq!(decision.source, DeciderSource::Decider, "{decision:?}");
        assert_eq!(
            decision.answer,
            DeciderAnswer::RunName {
                title: "Short run titles".into(),
                slug: "short-run-titles".into(),
            }
        );
    }
    let calls = fx.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| c["kind"] == "run_name"), "{calls:#?}");
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with("[anthrex decider] run_name v1\n"),
        "{prompt}"
    );
    assert!(prompt.ends_with("in anthrex, at the bottom I want a short title"));
}

#[test]
fn an_invalid_answer_or_a_failure_falls_back() {
    let fx = Fixture::new();
    fx.script(
        "run_name",
        1,
        json!({"answer": {"title": "Title", "slug": "title"}}),
    );
    fx.script(
        "run_name",
        2,
        json!({"answer": {"title": "Two words", "slug": "Bad Slug"}}),
    );
    let ctx = fx.context(DeciderMode::Claude);
    let ask = || {
        runtime()
            .block_on(decide_within(&ctx, &request("add login"), BOUND))
            .1
    };
    let fallback = DeciderAnswer::RunName {
        title: String::new(),
        slug: String::new(),
    };
    for (reason, why) in [
        (
            "the decider's answer does not match the schema: title: must have 2 to 8 words",
            "a single word",
        ),
        (
            "the decider's answer does not match the schema: slug: expected lowercase ASCII letters and digits joined by single -",
            "a bad slug",
        ),
        // No script left: `fake-agent` exits 2.
        ("the decider exited before answering (code 2)", "a failure"),
    ] {
        let decision = ask();
        assert_eq!(decision.source, DeciderSource::Fallback, "{why}");
        assert_eq!(decision.fallback_reason.as_deref(), Some(reason), "{why}");
        assert_eq!(decision.answer, fallback, "{why}");
    }
    // Off: the fallback at once, with nothing spawned.
    let calls = fx.calls().len();
    let (routed, decision) = runtime().block_on(decide_within(
        &fx.context(DeciderMode::Off),
        &request("add login"),
        BOUND,
    ));
    assert!(routed.is_none());
    assert_eq!(
        decision.fallback_reason.as_deref(),
        Some("deciders are off")
    );
    assert_eq!(fx.calls().len(), calls);
}

/// A decider that never answers is cut at the bound, below the context's own timeout,
/// and its process is gone. The wait is at least `BOUND` (the code's own budget) and
/// at most `BOUND` plus a spawn's slack (docs/timing-budgets.md).
#[test]
fn a_hanging_decider_is_cut_at_the_bound() {
    let fx = Fixture::new();
    fx.script("run_name", 1, json!({"hang": true}));
    let marker = fx.marker();
    let ctx = fx.context(DeciderMode::Claude);
    assert!(ctx.timeout > BOUND);
    let started = Instant::now();
    let (routed, decision) = runtime().block_on(decide_within(
        &ctx,
        &request(&format!("hang {marker}")),
        BOUND,
    ));
    let elapsed = started.elapsed();
    assert!(routed.is_some());
    assert_eq!(decision.source, DeciderSource::Fallback);
    assert_eq!(
        decision.fallback_reason.as_deref(),
        Some("the decider timed out after 2 s")
    );
    assert!(elapsed >= BOUND, "{elapsed:?}");
    assert!(elapsed < BOUND + SPAWN_SLACK, "{elapsed:?}");
    // Only look: the test signals nothing.
    assert_gone(&marker);
}
