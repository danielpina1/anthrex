use proto::{Effort, Runtime, Strength};

use super::*;

fn route(runtime: Runtime, model: &str, strength: Strength, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.to_string(),
        strength,
        effort,
    }
}

#[test]
fn pick_reviewer_prefers_the_other_runtime_at_or_above_the_author() {
    let roster = config::default_roster();
    let author = route(Runtime::Codex, "", Strength::Standard, Effort::Medium);

    let reviewer = pick_reviewer(&roster, &author, ReviewLevel::Medium);

    assert_eq!(reviewer.runtime, Runtime::Claude);
    assert_eq!(reviewer.model, "claude-sonnet-5");
    assert_eq!(reviewer.effort, Effort::Medium);
}

#[test]
fn small_level_takes_the_cheapest_other_runtime() {
    let roster = config::default_roster();
    let author = route(
        Runtime::Claude,
        "claude-haiku-4-5",
        Strength::Fast,
        Effort::Low,
    );

    let reviewer = pick_reviewer(&roster, &author, ReviewLevel::Small);

    assert_eq!(reviewer.runtime, Runtime::Codex);
    assert_eq!(reviewer.model, "");
    assert_eq!(reviewer.effort, Effort::Low);
}

#[test]
fn frontier_level_falls_back_to_the_same_runtime() {
    let roster = config::default_roster();

    let codex_author = route(Runtime::Codex, "", Strength::Standard, Effort::High);
    let reviewer = pick_reviewer(&roster, &codex_author, ReviewLevel::Frontier);
    assert_eq!(reviewer.runtime, Runtime::Claude);
    assert_eq!(reviewer.model, "claude-opus-5");
    assert_eq!(reviewer.effort, Effort::High);

    let claude_author = route(
        Runtime::Claude,
        "claude-opus-5",
        Strength::Frontier,
        Effort::High,
    );
    let reviewer = pick_reviewer(&roster, &claude_author, ReviewLevel::Frontier);
    assert_eq!(reviewer.runtime, Runtime::Claude);
    assert_eq!(reviewer.model, "claude-opus-5");
    assert_eq!(reviewer.effort, Effort::High);
}

#[test]
fn escalate_raises_effort_then_changes_runtime() {
    let default_roster = config::default_roster();

    // medium -> high, same runtime and model.
    let r = route(
        Runtime::Claude,
        "claude-sonnet-5",
        Strength::Standard,
        Effort::Medium,
    );
    let escalated = escalate(&default_roster, &r);
    assert_eq!(escalated.runtime, Runtime::Claude);
    assert_eq!(escalated.model, "claude-sonnet-5");
    assert_eq!(escalated.effort, Effort::High);

    // high Claude standard -> Codex "" high.
    let r = route(
        Runtime::Claude,
        "claude-sonnet-5",
        Strength::Standard,
        Effort::High,
    );
    let escalated = escalate(&default_roster, &r);
    assert_eq!(escalated.runtime, Runtime::Codex);
    assert_eq!(escalated.model, "");
    assert_eq!(escalated.effort, Effort::High);

    // high Claude claude-haiku-4-5 on a Claude-only roster -> claude-sonnet-5 high
    // (no peer, one strength up).
    let claude_only: Vec<ModelEntry> = default_roster
        .iter()
        .filter(|entry| entry.runtime == Runtime::Claude)
        .cloned()
        .collect();
    let r = route(
        Runtime::Claude,
        "claude-haiku-4-5",
        Strength::Fast,
        Effort::High,
    );
    let escalated = escalate(&claude_only, &r);
    assert_eq!(escalated.runtime, Runtime::Claude);
    assert_eq!(escalated.model, "claude-sonnet-5");
    assert_eq!(escalated.effort, Effort::High);

    // high Claude claude-opus-5 on the default roster -> unchanged (no Codex frontier
    // entry, nothing above frontier).
    let r = route(
        Runtime::Claude,
        "claude-opus-5",
        Strength::Frontier,
        Effort::High,
    );
    let escalated = escalate(&default_roster, &r);
    assert_eq!(escalated, r);
}
