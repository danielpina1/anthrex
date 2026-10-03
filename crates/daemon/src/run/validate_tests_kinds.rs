//! Milestone 9 decision 24: research and review tasks (task M9.4). They replace M8a's
//! `research_and_review_kinds_are_deferred`.

use proto::TestMode;

use crate::run::test_support::*;

fn one(id: &str, extra: &str) -> String {
    task_toml(id, "S", &format!("[\"crates/{id}/src/lib.rs\"]"), extra)
}

/// A research or review task, S, owning nothing; `extra` is appended.
fn reader(id: &str, kind: &str, extra: &str) -> String {
    task_toml(id, "S", "[]", &format!("kind = \"{kind}\"\n{extra}"))
}

const LEAVE_OWNS_EMPTY: &str =
    "research and review tasks change nothing; leave owns empty (rule 5.2)";

#[test]
fn research_task_with_owns_is_refused() {
    let text = plan_with(
        PROFILE,
        &[
            one("r1", "kind = \"research\""),
            one("v1", "kind = \"review\"\nreview_target = \"main\""),
        ],
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(Some("r1"), "owns", "5.2", LEAVE_OWNS_EMPTY),
            err(Some("v1"), "owns", "5.2", LEAVE_OWNS_EMPTY),
        ]
    );
}

#[test]
fn review_task_needs_a_review_target() {
    let text = plan_with(PROFILE, &[reader("v1", "review", "")]);
    assert_eq!(
        errors_of(&text),
        vec![err(
            Some("v1"),
            "review_target",
            "5.2",
            "required for a review task (rule 5.2)"
        )]
    );
    let run = run_ok(&plan_with(
        PROFILE,
        &[reader("v1", "review", "review_target = \"main\"")],
    ));
    assert_eq!(task(&run, "v1").spec.review_target.as_deref(), Some("main"));
}

#[test]
fn review_target_syntax() {
    let long = "a".repeat(201);
    // The M9.4 review fixes (ruling 6): no part is empty or starts with `.`, and the
    // whole target is at most 200 characters, as the MCP schema has it.
    let long_range = format!("{}..{}", "a".repeat(100), "b".repeat(99));
    let full_range = format!("{}..{}", "a".repeat(100), "b".repeat(98));
    let cases: [(&str, bool); 19] = [
        ("main", true),
        ("a1b2c3d", true),
        ("main..feature/x", true),
        ("HEAD~3..HEAD", true),
        ("a..b", true),
        (&full_range, true),
        ("a...b", false),
        (".a..b", false),
        ("a..", false),
        ("..b", false),
        (".main", false),
        (&long_range, false),
        ("v1.2@{1}^", false),
        ("-x", false),
        ("a..-x", false),
        ("a..b..c", false),
        ("a b", false),
        ("", false),
        (&long, false),
    ];
    for (target, ok) in cases {
        let text = plan_with(
            PROFILE,
            &[reader(
                "v1",
                "review",
                &format!("review_target = \"{target}\""),
            )],
        );
        let expected = if ok {
            vec![]
        } else {
            vec![err(
                Some("v1"),
                "review_target",
                "5.2",
                &format!("{target} is not a revision or a range <a>..<b>"),
            )]
        };
        assert_eq!(
            build(&text).err().unwrap_or_default(),
            expected,
            "{target:?}"
        );
    }
    assert_eq!(full_range.len(), 200);
    assert_eq!(long_range.len(), 201);
}

#[test]
fn code_task_with_review_target_is_refused() {
    let text = plan_with(
        PROFILE,
        &[
            one("t1", "review_target = \"main\""),
            reader("r1", "research", "review_target = \"main\""),
        ],
    );
    let only = "only review tasks have a review target";
    assert_eq!(
        errors_of(&text),
        vec![
            err(Some("t1"), "review_target", "5.2", only),
            err(Some("r1"), "review_target", "5.2", only),
        ]
    );
}

#[test]
fn research_and_review_force_test_mode_none_with_the_note() {
    let note = "test mode none: research and review tasks change nothing (rule 8)";
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            // No reason is required, and a declared tdd is overridden.
            reader("r1", "research", "test_mode = \"tdd\""),
            reader(
                "v1",
                "review",
                "review_target = \"main\"\ntest_mode = \"check\"\ntest_mode_reason = \"read only\"",
            ),
        ],
    ));
    for id in ["r1", "v1"] {
        let t = task(&run, id);
        assert_eq!(t.test_mode, TestMode::None, "{id}");
        assert_eq!(t.notes, vec![note.to_string()], "{id}");
    }
    // A given reason is kept.
    assert_eq!(
        task(&run, "v1").spec.test_mode_reason.as_deref(),
        Some("read only")
    );
    // Size is still required and still sets the budget.
    assert_eq!(task(&run, "r1").size, proto::Size::S);
}

/// Milestone 9.5 task 2: `race` and `pair` parse (protocol 16) but are refused until
/// task M9.5.14 validates them.
#[test]
fn race_and_pair_are_not_available_yet() {
    let text = plan_with(
        PROFILE,
        &[
            one("t1", "race = true"),
            one("t2", "pair = true"),
            one("t3", "race = false\npair = false"),
        ],
    );
    let errors = errors_of(&text);
    assert_eq!(
        errors.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "task t1: race: not available yet",
            "task t2: pair: not available yet"
        ]
    );
}
