//! Plan and edit parsing, `PlanError` display, and decision 15's slug.

use super::super::*;
use crate::run::test_support::*;

#[test]
fn plan_error_display() {
    assert_eq!(
        err(
            Some("t1"),
            "size",
            "7.2.4",
            "L tasks are never executed; split the task (rule 7.2.4)"
        )
        .to_string(),
        "task t1: size: L tasks are never executed; split the task (rule 7.2.4)"
    );
    assert_eq!(
        err(None, "deps", "12.1", "cycle t1 -> t2 -> t1").to_string(),
        "deps: cycle t1 -> t2 -> t1"
    );
}

#[test]
fn parse_plan_names_line_and_key() {
    let text = "goal = \"g\"\n\n[[task]]\nid = \"t1\"\nbogus = 1\n";
    let error = parse_plan(text).unwrap_err();
    assert!(error.contains("line 5"), "{error}");
    assert!(error.contains("bogus"), "{error}");
}

#[test]
fn parse_edits_reads_an_edit_file() {
    let text = "[[edit]]\nop = \"cancel_task\"\ntask_id = \"t2\"\n\n[[edit]]\nop = \"pause\"\n";
    assert_eq!(
        parse_edits(text).unwrap(),
        vec![
            PlanEdit::CancelTask {
                task_id: "t2".to_string()
            },
            PlanEdit::Pause,
        ]
    );
    assert!(parse_edits("[[edit]]\nop = \"explode\"\n").is_err());
}

#[test]
fn slug_examples() {
    assert_eq!(
        slug("Add password reset", 0x3f9a),
        "add-password-reset-3f9a"
    );
    assert_eq!(slug("  Fix: the ÄPI!! ", 1), "fix-the-pi-0001");
    assert_eq!(slug("!!!", 2), "run-0002");

    // 60 characters whose word boundaries do not fall on 32: the cut lands mid-word
    // ("...-imp" is cut from "improve"). Each character is one byte after mapping
    // (only ASCII alphanumerics and `-` survive), so the 32-character cut is a 32-byte
    // cut; 32 is not a multiple of any word length here.
    let goal = "Refactor the scheduler loop to improve fairness across tasks";
    assert_eq!(goal.chars().count(), 60);
    let s = slug(goal, 0xabcd);
    let (head, suffix) = s.rsplit_once('-').unwrap();
    assert_eq!(suffix, "abcd");
    assert_eq!(head, "refactor-the-scheduler-loop-to-i");
    assert_eq!(head.len(), 32);
    assert!(head.len() <= 32 && !head.ends_with('-'));
}

#[test]
fn slug_cut_on_a_word_boundary() {
    // The 32nd character is the last of a word and the 33rd is `-`: exactly 32 kept.
    let goal = "aaaaaaaa bbbbbbbb cccccccc ddddd eeee";
    assert_eq!(slug(goal, 0x10), "aaaaaaaa-bbbbbbbb-cccccccc-ddddd-0010");
    assert_eq!("aaaaaaaa-bbbbbbbb-cccccccc-ddddd".len(), 32);
    // The 32nd character is `-` itself: the cut would end in `-`, which is trimmed,
    // leaving 31.
    let goal = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbb";
    let s = slug(goal, 0x10);
    assert_eq!(s, format!("{}-0010", "a".repeat(31)));
}

#[test]
fn random_suffixes_vary() {
    let draws: std::collections::BTreeSet<u16> = (0..16).map(|_| random_suffix()).collect();
    assert!(draws.len() > 1, "{draws:?}");
}
