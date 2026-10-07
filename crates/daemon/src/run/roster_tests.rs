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
    let author = route(Runtime::Codex, "", Strength::Standard, Effort::MEDIUM);

    let reviewer = pick_reviewer(&roster, &author, ReviewLevel::Medium);

    assert_eq!(reviewer.runtime, Runtime::Claude);
    assert_eq!(reviewer.model, "claude-sonnet-5");
    assert_eq!(reviewer.effort, Effort::MEDIUM);
}

#[test]
fn small_level_takes_the_cheapest_other_runtime() {
    let roster = config::default_roster();
    let author = route(
        Runtime::Claude,
        "claude-haiku-4-5",
        Strength::Fast,
        Effort::LOW,
    );

    let reviewer = pick_reviewer(&roster, &author, ReviewLevel::Small);

    assert_eq!(reviewer.runtime, Runtime::Codex);
    assert_eq!(reviewer.model, "");
    assert_eq!(reviewer.effort, Effort::LOW);
}

#[test]
fn frontier_level_falls_back_to_the_same_runtime() {
    let roster = config::default_roster();

    let codex_author = route(Runtime::Codex, "", Strength::Standard, Effort::HIGH);
    let reviewer = pick_reviewer(&roster, &codex_author, ReviewLevel::Frontier);
    assert_eq!(reviewer.runtime, Runtime::Claude);
    assert_eq!(reviewer.model, "claude-opus-5-5");
    assert_eq!(reviewer.effort, Effort::HIGH);

    let claude_author = route(
        Runtime::Claude,
        "claude-opus-5-5",
        Strength::Frontier,
        Effort::HIGH,
    );
    let reviewer = pick_reviewer(&roster, &claude_author, ReviewLevel::Frontier);
    assert_eq!(reviewer.runtime, Runtime::Claude);
    assert_eq!(reviewer.model, "claude-opus-5-5");
    assert_eq!(reviewer.effort, Effort::HIGH);
}

/// Pins review finding 2 / ruling I2: decision 35's "preferring a model different from
/// the author's" is read as a hard exclusion, so a stronger same-runtime model at a
/// higher strength is picked before falling back to the author's own model. On a
/// single-runtime roster spanning all three strengths, a `standard` author reviewed at
/// `Medium` (required strength = standard) gets the `frontier` model, not
/// `claude-sonnet-5` itself, because the peer runtime has no entries and the
/// same-runtime search excludes the author's own model before it ever considers
/// strength.
#[test]
fn pick_reviewer_prefers_a_stronger_model_over_the_authors_own() {
    let claude_only = s(&[
        (Runtime::Claude, "claude-haiku-4-5", Strength::Fast),
        (Runtime::Claude, "claude-sonnet-5", Strength::Standard),
        (Runtime::Claude, "claude-opus-5", Strength::Frontier),
    ]);
    let author = route(
        Runtime::Claude,
        "claude-sonnet-5",
        Strength::Standard,
        Effort::MEDIUM,
    );

    let reviewer = pick_reviewer(&claude_only, &author, ReviewLevel::Medium);

    assert_eq!(reviewer.runtime, Runtime::Claude);
    assert_eq!(reviewer.model, "claude-opus-5");
    assert_eq!(reviewer.effort, Effort::MEDIUM);
}

fn s(entries: &[(Runtime, &str, Strength)]) -> Vec<ModelEntry> {
    entries
        .iter()
        .map(|(runtime, model, strength)| ModelEntry {
            runtime: *runtime,
            model: model.to_string(),
            strength: *strength,
            note: String::new(),
        })
        .collect()
}

/// Milestone 9.6 decision 10 (task M9.6.8, carry M-4): the strongest roster entry of a
/// runtime, `frontier` first and the first in roster order among ties.
#[test]
fn strongest_of_takes_the_strongest_first_in_roster_order() {
    let entry = |runtime, model: &str, strength| ModelEntry {
        runtime,
        model: model.into(),
        strength,
        note: String::new(),
    };
    let roster = vec![
        entry(Runtime::Codex, "c-standard", Strength::Standard),
        entry(Runtime::Claude, "a-standard", Strength::Standard),
        entry(Runtime::Codex, "c-frontier-1", Strength::Frontier),
        entry(Runtime::Codex, "c-frontier-2", Strength::Frontier),
        entry(Runtime::Claude, "a-fast", Strength::Fast),
        entry(Runtime::Claude, "a-standard-2", Strength::Standard),
    ];
    let model = |runtime| strongest_of(&roster, runtime).map(|e| e.model.as_str());
    assert_eq!(model(Runtime::Codex), Some("c-frontier-1"));
    assert_eq!(model(Runtime::Claude), Some("a-standard"));
    assert_eq!(model(Runtime::Shell), None);
}
