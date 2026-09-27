//! M8c.9: the task edit form's pure rules (decision 33).

use super::*;
use crate::tree::run_fixtures::{RUN_ID, task};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Effort, Route, RouteSpec, Runtime, Size, Strength, TaskInfo, TaskState, TestMode};

/// The brief's `t1`: route spec `{ claude, policy, policy, medium }`, resolved to
/// Claude, `claude-sonnet-5`, standard, medium; M, tdd, brief `Line one\nLine two`.
pub(crate) fn edit_fixture_task() -> TaskInfo {
    let mut t1 = task("t1", "reset token model", Size::M, TaskState::Pending);
    t1.route_spec = RouteSpec {
        runtime: Some(Runtime::Claude),
        model: None,
        strength: None,
        effort: Some(Effort::Medium),
    };
    t1.route = Route {
        runtime: Runtime::Claude,
        model: "claude-sonnet-5".into(),
        strength: Strength::Standard,
        effort: Effort::Medium,
    };
    t1.brief = "Line one\nLine two".into();
    t1
}

pub(crate) fn edit_fixture_form() -> TaskEditForm {
    TaskEditForm::new(RUN_ID, &edit_fixture_task())
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn focus(form: &mut TaskEditForm, field: EditField) {
    for _ in 0..16 {
        if form.focus == field {
            return;
        }
        assert_eq!(form.on_key(key(KeyCode::Tab)), EditOutcome::Stay);
    }
    panic!("{field:?} is not reachable with Tab");
}

fn typed(form: &mut TaskEditForm, text: &str) {
    for c in text.chars() {
        assert_eq!(form.on_key(key(KeyCode::Char(c))), EditOutcome::Stay);
    }
}

fn amend(form: &TaskEditForm) -> PlanEdit {
    let mut edits = form.edits().expect("valid");
    assert_eq!(edits.len(), 1, "{edits:?}");
    edits.remove(0)
}

#[test]
fn visible_fields_hide_the_reason_for_tdd() {
    let mut form = edit_fixture_form();
    use EditField::*;
    assert_eq!(
        form.visible_fields(),
        vec![Runtime, Model, Strength, Effort, Size, TestMode, Brief]
    );
    form.test_mode = proto::TestMode::Check;
    assert_eq!(
        form.visible_fields(),
        vec![
            Runtime, Model, Strength, Effort, Size, TestMode, Reason, Brief
        ]
    );
    form.test_mode = proto::TestMode::None;
    assert!(form.visible_fields().contains(&Reason));
}

#[test]
fn tab_and_shift_tab_walk_the_visible_fields_and_wrap() {
    let mut form = edit_fixture_form();
    assert_eq!(form.focus, EditField::Runtime);
    form.on_key(key(KeyCode::BackTab));
    assert_eq!(form.focus, EditField::Brief);
    form.on_key(key(KeyCode::Down));
    assert_eq!(form.focus, EditField::Runtime);
    form.on_key(key(KeyCode::Down));
    assert_eq!(form.focus, EditField::Model);
    form.on_key(key(KeyCode::Up));
    assert_eq!(form.focus, EditField::Runtime);
}

#[test]
fn choices_cycle_both_ways() {
    let mut form = edit_fixture_form();
    // Runtime: policy → claude → codex → policy.
    let mut seen = vec![];
    for _ in 0..3 {
        form.on_key(key(KeyCode::Right));
        seen.push(form.runtime);
    }
    assert_eq!(
        seen,
        vec![Some(Runtime::Codex), None, Some(Runtime::Claude)]
    );
    let mut seen = vec![];
    for _ in 0..3 {
        form.on_key(key(KeyCode::Left));
        seen.push(form.runtime);
    }
    assert_eq!(
        seen,
        vec![None, Some(Runtime::Codex), Some(Runtime::Claude)]
    );
    form.on_key(key(KeyCode::Char(' ')));
    assert_eq!(form.runtime, Some(Runtime::Codex), "Space cycles forward");

    focus(&mut form, EditField::Strength);
    let mut seen = vec![];
    for _ in 0..4 {
        form.on_key(key(KeyCode::Right));
        seen.push(form.strength);
    }
    use proto::Strength::*;
    assert_eq!(seen, vec![Some(Fast), Some(Standard), Some(Frontier), None]);
    let mut seen = vec![];
    for _ in 0..4 {
        form.on_key(key(KeyCode::Left));
        seen.push(form.strength);
    }
    assert_eq!(seen, vec![Some(Frontier), Some(Standard), Some(Fast), None]);

    focus(&mut form, EditField::Effort);
    let mut seen = vec![];
    for _ in 0..4 {
        form.on_key(key(KeyCode::Right));
        seen.push(form.effort);
    }
    use proto::Effort::*;
    assert_eq!(seen, vec![Some(High), None, Some(Low), Some(Medium)]);
    let mut seen = vec![];
    for _ in 0..4 {
        form.on_key(key(KeyCode::Left));
        seen.push(form.effort);
    }
    assert_eq!(seen, vec![Some(Low), None, Some(High), Some(Medium)]);

    focus(&mut form, EditField::Size);
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.size, Size::S);
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.size, Size::M);
    form.on_key(key(KeyCode::Left));
    assert_eq!(form.size, Size::S);
    form.on_key(key(KeyCode::Char(' ')));
    assert_eq!(form.size, Size::M);

    focus(&mut form, EditField::TestMode);
    let mut seen = vec![];
    for _ in 0..3 {
        form.on_key(key(KeyCode::Right));
        seen.push(form.test_mode);
    }
    assert_eq!(seen, vec![TestMode::Check, TestMode::None, TestMode::Tdd]);
    let mut seen = vec![];
    for _ in 0..3 {
        form.on_key(key(KeyCode::Left));
        seen.push(form.test_mode);
    }
    assert_eq!(seen, vec![TestMode::None, TestMode::Check, TestMode::Tdd]);
}

#[test]
fn an_l_task_steps_into_s_and_m() {
    let mut t = edit_fixture_task();
    t.size = Size::L;
    let mut form = TaskEditForm::new(RUN_ID, &t);
    focus(&mut form, EditField::Size);
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.size, Size::S);
    let mut form = TaskEditForm::new(RUN_ID, &t);
    focus(&mut form, EditField::Size);
    form.on_key(key(KeyCode::Left));
    assert_eq!(form.size, Size::M);
}

#[test]
fn esc_and_ctrl_c_cancel() {
    for cancel in [key(KeyCode::Esc), ctrl('c'), ctrl('C')] {
        let mut form = edit_fixture_form();
        assert_eq!(form.on_key(cancel), EditOutcome::Cancel);
        // Also while submitting, the only keys that still do anything.
        let mut form = edit_fixture_form();
        form.on_key(key(KeyCode::Right));
        assert!(matches!(
            form.on_key(key(KeyCode::Enter)),
            EditOutcome::Submit(_)
        ));
        assert_eq!(form.on_key(cancel), EditOutcome::Cancel);
    }
    // A plain `c` is typed, not a cancel.
    let mut form = edit_fixture_form();
    focus(&mut form, EditField::Model);
    assert_eq!(form.on_key(key(KeyCode::Char('c'))), EditOutcome::Stay);
    assert_eq!(form.model.text(), "c");
}

#[test]
fn a_second_enter_while_submitting_sends_nothing() {
    let mut form = edit_fixture_form();
    focus(&mut form, EditField::Size);
    form.on_key(key(KeyCode::Right));
    focus(&mut form, EditField::Brief);
    assert!(matches!(
        form.on_key(key(KeyCode::Enter)),
        EditOutcome::Submit(_)
    ));
    assert!(form.submitting);
    assert_eq!(form.on_key(key(KeyCode::Enter)), EditOutcome::Stay);
    assert_eq!(form.on_key(key(KeyCode::Char('q'))), EditOutcome::Stay);
    assert_eq!(form.on_key(key(KeyCode::Tab)), EditOutcome::Stay);
    assert_eq!(
        form.focus,
        EditField::Brief,
        "keys do nothing while submitting"
    );
    form.on_paste("zzz");
    assert_eq!(form.brief.text(), "Line one↵Line two");
}

#[test]
fn only_the_reason_changing_sends_only_the_reason() {
    let mut t = edit_fixture_task();
    t.test_mode = TestMode::Check;
    t.test_mode_reason = Some("renames".into());
    let mut form = TaskEditForm::new(RUN_ID, &t);
    focus(&mut form, EditField::Reason);
    typed(&mut form, " only");
    assert_eq!(
        amend(&form),
        PlanEdit::AmendTask {
            task_id: "t1".into(),
            brief: None,
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: Some("renames only".into()),
            priority: None,
            size: None,
        }
    );
}

#[test]
fn back_to_tdd_sends_no_reason() {
    let mut t = edit_fixture_task();
    t.test_mode = TestMode::Check;
    t.test_mode_reason = Some("renames".into());
    let mut form = TaskEditForm::new(RUN_ID, &t);
    focus(&mut form, EditField::TestMode);
    form.on_key(key(KeyCode::Left));
    assert_eq!(form.test_mode, TestMode::Tdd);
    let PlanEdit::AmendTask {
        test_mode,
        test_mode_reason,
        ..
    } = amend(&form)
    else {
        panic!("an amend");
    };
    assert_eq!(test_mode, Some(TestMode::Tdd));
    assert_eq!(test_mode_reason, None);
}

#[test]
fn an_unchanged_check_task_with_no_reason_can_still_edit_its_brief() {
    let mut t = edit_fixture_task();
    t.test_mode = TestMode::Check;
    t.test_mode_reason = None;
    let mut form = TaskEditForm::new(RUN_ID, &t);
    focus(&mut form, EditField::Brief);
    typed(&mut form, "!");
    let PlanEdit::AmendTask {
        brief,
        test_mode,
        test_mode_reason,
        ..
    } = amend(&form)
    else {
        panic!("an amend");
    };
    assert_eq!(brief.as_deref(), Some("Line one\nLine two!"));
    assert_eq!((test_mode, test_mode_reason), (None, None));
}

#[test]
fn a_whitespace_reason_is_blank() {
    let mut form = edit_fixture_form();
    focus(&mut form, EditField::TestMode);
    form.on_key(key(KeyCode::Right));
    focus(&mut form, EditField::Reason);
    typed(&mut form, "   ");
    assert_eq!(
        form.edits(),
        Err((EditField::Reason, REASON_REQUIRED.to_string()))
    );
}

#[test]
fn the_model_is_sent_trimmed_and_blank_is_policy() {
    let mut t = edit_fixture_task();
    t.route_spec.model = Some("m1".into());
    let mut form = TaskEditForm::new(RUN_ID, &t);
    focus(&mut form, EditField::Model);
    assert_eq!(form.model.text(), "m1");
    form.on_key(ctrl('u'));
    typed(&mut form, "  m2 ");
    let PlanEdit::AmendTask { route, .. } = amend(&form) else {
        panic!("an amend");
    };
    assert_eq!(route.expect("a route").model.as_deref(), Some("m2"));
    form.on_key(ctrl('u'));
    typed(&mut form, "   ");
    let PlanEdit::AmendTask { route, .. } = amend(&form) else {
        panic!("an amend");
    };
    assert_eq!(route.expect("a route").model, None, "blank is policy");
}

#[test]
fn a_paste_of_a_megabyte_is_bounded() {
    let mut form = edit_fixture_form();
    focus(&mut form, EditField::Brief);
    let huge = "word\n".repeat(200_000);
    assert_eq!(huge.len(), 1_000_000);
    form.on_paste(&huge);
    assert_eq!(form.brief.text().chars().count(), TEXT_MAX_CHARS);
    assert!(form.brief.text().starts_with("Line one↵Line twoword↵word↵"));
    // A full field takes no more typing either.
    typed(&mut form, "xyz");
    assert_eq!(form.brief.text().chars().count(), TEXT_MAX_CHARS);
    assert!(!form.brief.text().ends_with('z'));
}

#[test]
fn control_characters_in_a_pasted_model_are_dropped() {
    let mut form = edit_fixture_form();
    focus(&mut form, EditField::Model);
    form.on_paste("gpt\u{1b}[31m-5\r\n\u{7}\u{0}x\ty");
    assert_eq!(form.model.text(), "gpt[31m-5x y");
    focus(&mut form, EditField::Brief);
    form.on_key(key(KeyCode::End));
    form.on_paste("a\u{1b}b\r\nc\rd\te");
    assert_eq!(form.brief.text(), "Line one↵Line twoab↵c↵d e");
}

#[test]
fn a_paste_on_a_choice_field_changes_nothing() {
    let mut form = edit_fixture_form();
    let before = form.clone();
    form.on_paste("codex");
    assert_eq!(form, before);
}

#[test]
fn hostile_briefs_open_cleaned_and_unchanged() {
    let mut t = edit_fixture_task();
    t.brief = "a\r\nb\u{1b}[2Jc\td".into();
    let form = TaskEditForm::new(RUN_ID, &t);
    assert_eq!(form.brief.text(), "a↵b[2Jc d");
    assert_eq!(form.edits(), Ok(vec![]), "opening changes nothing");
}

#[test]
fn value_parts_show_policy_with_the_resolved_value() {
    let form = edit_fixture_form();
    assert_eq!(
        form.value_parts(EditField::Runtime),
        ("‹ claude ›".into(), None)
    );
    assert_eq!(
        form.value_parts(EditField::Model),
        ("policy".into(), Some("claude-sonnet-5".into()))
    );
    assert_eq!(
        form.value_parts(EditField::Strength),
        ("‹ policy ›".into(), Some("standard".into()))
    );
    assert_eq!(
        form.value_parts(EditField::Effort),
        ("‹ medium ›".into(), None)
    );
    assert_eq!(form.value_parts(EditField::Size), ("‹ M ›".into(), None));
    assert_eq!(
        form.value_parts(EditField::TestMode),
        ("‹ tdd ›".into(), None)
    );
    assert_eq!(
        form.value_parts(EditField::Brief),
        ("Line one↵Line two".into(), None)
    );
}

#[test]
fn a_pinned_strength_or_effort_back_to_policy_sends_none() {
    let mut t = edit_fixture_task();
    t.route_spec.strength = Some(Strength::Frontier);
    let mut form = TaskEditForm::new(RUN_ID, &t);
    focus(&mut form, EditField::Strength);
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.strength, None);
    focus(&mut form, EditField::Effort);
    form.on_key(key(KeyCode::Left));
    form.on_key(key(KeyCode::Left));
    assert_eq!(form.effort, None);
    let PlanEdit::AmendTask { route, .. } = amend(&form) else {
        panic!("an amend");
    };
    assert_eq!(
        route,
        Some(RouteSpec {
            runtime: Some(Runtime::Claude),
            model: None,
            strength: None,
            effort: None,
        }),
        "policy is sent as policy, never as the value it resolved to"
    );
}
