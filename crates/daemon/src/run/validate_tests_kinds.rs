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
    let cases: [(&str, bool); 11] = [
        ("main", true),
        ("a1b2c3d", true),
        ("main..feature/x", true),
        ("HEAD~3..HEAD", true),
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
    // Two full-length parts are accepted.
    let two = format!("{}..{}", "a".repeat(200), "b".repeat(200));
    let text = plan_with(
        PROFILE,
        &[reader(
            "v1",
            "review",
            &format!("review_target = \"{two}\""),
        )],
    );
    assert!(build(&text).is_ok());
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
