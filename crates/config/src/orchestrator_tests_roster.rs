//! `[[orchestrator.models]]` parsing: invalid entries, the built-in roster merge, and
//! the empty-result fallback. Split out of `orchestrator_tests.rs` to keep that file
//! under the 600-line rule.

use super::*;

#[test]
fn invalid_model_entries_are_skipped_with_their_index() {
    let note_81 = "x".repeat(81);
    let text = format!(
        r#"
[[orchestrator.models]]
runtime = "perl"
model = "whatever"
strength = "standard"

[[orchestrator.models]]
runtime = "claude"
model = ""
strength = "standard"

[[orchestrator.models]]
runtime = "claude"
model = "claude-sonnet-5"
strength = "huge"

[[orchestrator.models]]
runtime = "claude"
model = "claude-sonnet-5"
strength = "standard"
note = "{note_81}"
"#
    );
    let (config, problems) = parse(&text);

    assert_eq!(
        keys(&problems),
        HashSet::from([
            "orchestrator.models[0]".to_string(),
            "orchestrator.models[1]".to_string(),
            "orchestrator.models[2]".to_string(),
            "orchestrator.models[3]".to_string(),
        ])
    );
    // Every entry was invalid and skipped; `builtin_models` defaults to true, so the
    // untouched built-in roster is what is left -- not empty, so the separate
    // empty-result fallback (`no_valid_models_uses_the_default_roster`) does not fire.
    assert_eq!(config.orchestrator.models, default_roster());
}

#[test]
fn user_models_extend_and_replace_builtins() {
    let (config, problems) = parse(
        r#"
[[orchestrator.models]]
runtime = "claude"
model = "claude-sonnet-5"
strength = "frontier"
note = "promoted"
"#,
    );
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    let models = &config.orchestrator.models;
    assert_eq!(models.len(), 4);
    // Replaced in place: still the second entry, same runtime and model, new strength.
    assert_eq!(models[1].runtime, proto::Runtime::Claude);
    assert_eq!(models[1].model, "claude-sonnet-5");
    assert_eq!(models[1].strength, proto::Strength::Frontier);
    assert_eq!(models[1].note, "promoted");
    // The other three built-ins are untouched.
    assert_eq!(models[0], default_roster()[0]);
    assert_eq!(models[2], default_roster()[2]);
    assert_eq!(models[3], default_roster()[3]);
}

#[test]
fn builtin_models_false_drops_builtins() {
    let (config, problems) = parse(
        r#"
[orchestrator]
builtin_models = false

[[orchestrator.models]]
runtime = "codex"
model = "my-model"
strength = "standard"
"#,
    );
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    assert_eq!(
        config.orchestrator.models,
        vec![proto::ModelEntry {
            runtime: proto::Runtime::Codex,
            model: "my-model".to_string(),
            strength: proto::Strength::Standard,
            note: String::new(),
        }]
    );
}

#[test]
fn no_valid_models_uses_the_default_roster() {
    let (config, problems) = parse(
        r#"
[orchestrator]
builtin_models = false
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.models".to_string()])
    );
    assert_eq!(config.orchestrator.models, default_roster());
}
