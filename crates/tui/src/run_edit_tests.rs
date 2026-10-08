//! M8c.9: the task edit form's pure rules (decision 33).

use super::*;
use crate::tree::run_fixtures::{RUN_ID, task};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Effort, Route, RouteSpec, Runtime, Size, TaskInfo, TaskState, TestMode};

/// The brief's `t1`: route spec `{ claude, policy, policy, medium }`, resolved to
/// Claude, `claude-sonnet-5`, standard, medium; M, tdd, brief `Line one\nLine two`.
pub(crate) fn edit_fixture_task() -> TaskInfo {
    let mut t1 = task("t1", "reset token model", Size::M, TaskState::Pending);
    t1.route_spec = RouteSpec {
        runtime: Some(Runtime::Claude),
        model: None,
        strength: None,
        effort: Some(Effort::MEDIUM),
    };
    t1.route = Route {
        runtime: Runtime::Claude,
        model: "claude-sonnet-5".into(),
        effort: Effort::MEDIUM,
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
    // Milestone 9.8 decision 39 (changed expectation): `model` and `effort` in place of
    // `runtime`, `model`, `strength` and `effort`.
    assert_eq!(
        form.visible_fields(),
        vec![Model, Effort, Size, TestMode, Brief]
    );
    form.test_mode = proto::TestMode::Check;
    assert_eq!(
        form.visible_fields(),
        vec![Model, Effort, Size, TestMode, Reason, Brief]
    );
    form.test_mode = proto::TestMode::None;
    assert!(form.visible_fields().contains(&Reason));
}

#[test]
fn tab_and_shift_tab_walk_the_visible_fields_and_wrap() {
    let mut form = edit_fixture_form();
    assert_eq!(form.focus, EditField::Model);
    form.on_key(key(KeyCode::BackTab));
    assert_eq!(form.focus, EditField::Brief);
    form.on_key(key(KeyCode::Down));
    assert_eq!(form.focus, EditField::Model);
    form.on_key(key(KeyCode::Down));
    assert_eq!(form.focus, EditField::Effort);
    form.on_key(key(KeyCode::Up));
    assert_eq!(form.focus, EditField::Model);
}

#[test]
fn choices_cycle_both_ways() {
    let mut form = edit_fixture_form();
    // Milestone 9.8 decision 39 (changed expectation): the runtime and strength cycles
    // are gone; the effort cycles the route model's catalog efforts (here three), then
    // the role table's.
    form.efforts = ["low", "medium", "high"].map(String::from).to_vec();
    focus(&mut form, EditField::Effort);
    let mut seen = vec![];
    for _ in 0..4 {
        form.on_key(key(KeyCode::Right));
        seen.push(form.route.effort.clone());
    }
    use proto::Effort;
    assert_eq!(
        seen,
        vec![
            Some(Effort::HIGH),
            None,
            Some(Effort::LOW),
            Some(Effort::MEDIUM)
        ]
    );
    let mut seen = vec![];
    for _ in 0..4 {
        form.on_key(key(KeyCode::Left));
        seen.push(form.route.effort.clone());
    }
    assert_eq!(
        seen,
        vec![
            Some(Effort::LOW),
            None,
            Some(Effort::HIGH),
            Some(Effort::MEDIUM)
        ]
    );

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
        focus(&mut form, EditField::Size);
        form.on_key(key(KeyCode::Right));
        assert!(matches!(
            form.on_key(key(KeyCode::Enter)),
            EditOutcome::Submit(_)
        ));
        assert_eq!(form.on_key(cancel), EditOutcome::Cancel);
    }
    // A plain `c` is typed, not a cancel.
    let mut form = edit_fixture_form();
    focus(&mut form, EditField::Brief);
    form.on_key(key(KeyCode::End));
    assert_eq!(form.on_key(key(KeyCode::Char('c'))), EditOutcome::Stay);
    assert_eq!(form.brief.text(), "Line one\nLine twoc");
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
    assert_eq!(form.brief.text(), "Line one\nLine two");
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
            deps: None,
            stage: None,
            race: None,
            pair: None,
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
fn a_paste_of_a_megabyte_is_bounded() {
    // A one-line field (the reason, since milestone 9.8 the only one) stops at
    // `TEXT_MAX_CHARS`, and takes no more typing.
    let mut form = edit_fixture_form();
    form.test_mode = TestMode::Check;
    focus(&mut form, EditField::Reason);
    form.on_paste(&"m".repeat(1_000_000));
    assert_eq!(form.reason.text().chars().count(), TEXT_MAX_CHARS);
    typed(&mut form, "xyz");
    assert_eq!(form.reason.text().chars().count(), TEXT_MAX_CHARS);
    assert!(!form.reason.text().ends_with('z'));
    // The brief holds up to `BRIEF_MAX_CHARS` (decision 35; one million characters
    // since the final fix wave): nearly a megabyte goes in whole, and no more after it.
    focus(&mut form, EditField::Brief);
    let huge = "word\n".repeat(199_000);
    assert_eq!(huge.len(), 995_000);
    form.on_paste(&huge);
    assert_eq!(form.brief.text(), format!("Line one\nLine two{huge}"));
    form.on_paste(&"z".repeat(10_000));
    assert_eq!(form.brief.text().chars().count(), BRIEF_MAX_CHARS);
}

#[test]
fn control_characters_in_a_pasted_reason_are_dropped() {
    let mut form = edit_fixture_form();
    form.test_mode = TestMode::Check;
    focus(&mut form, EditField::Reason);
    form.on_paste("gpt\u{1b}[31m-5\r\n\u{7}\u{0}x\ty");
    assert_eq!(form.reason.text(), "gpt[31m-5x y");
    focus(&mut form, EditField::Brief);
    form.on_key(key(KeyCode::End));
    form.on_paste("a\u{1b}b\r\nc\rd\te");
    assert_eq!(form.brief.text(), "Line one\nLine twoab\nc\nd e");
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
    assert_eq!(form.brief.text(), "a\nb[2Jc d");
    assert_eq!(form.edits(), Ok(vec![]), "opening changes nothing");
}

#[test]
fn value_parts_show_the_role_table_with_the_resolved_value() {
    let form = edit_fixture_form();
    // Milestone 9.8 (changed expectation): the plan names the runtime only, so the model
    // is the table's on it; `policy` reads `role table`.
    assert_eq!(
        form.value_parts(EditField::Model),
        (
            "‹ Claude · role table ›".into(),
            Some("claude-sonnet-5".into())
        )
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
        ("Line one\nLine two".into(), None)
    );
    let mut t = edit_fixture_task();
    t.route_spec = RouteSpec::default();
    let form = TaskEditForm::new(RUN_ID, &t);
    // Fix round 1 (controller ruling I2): the table's model and effort, named.
    assert_eq!(
        form.value_parts(EditField::Model),
        (
            "‹ role table (Claude · claude-sonnet-5 · medium) ›".into(),
            None
        )
    );
    assert_eq!(
        form.value_parts(EditField::Effort),
        ("‹ role table ›".into(), Some("medium".into()))
    );
}

#[test]
fn a_pinned_effort_back_to_the_role_table_sends_none() {
    let mut form = edit_fixture_form();
    form.efforts = ["low", "medium", "high"].map(String::from).to_vec();
    focus(&mut form, EditField::Effort);
    form.on_key(key(KeyCode::Left));
    form.on_key(key(KeyCode::Left));
    assert_eq!(form.route.effort, None);
    let PlanEdit::AmendTask { route, .. } = amend(&form) else {
        panic!("an amend");
    };
    assert_eq!(
        route,
        // Fix round 1 (review minor 4): with no model the legacy runtime goes too.
        Some(RouteSpec::default()),
        "the role table's is sent as none, never as the value it resolved to"
    );
}

#[test]
fn a_pinned_model_is_kept_byte_for_byte_through_an_effort_change() {
    for hostile in [false, true] {
        let pinned = if hostile {
            " m\u{1b}x "
        } else {
            "claude-opus-9"
        };
        let mut t = edit_fixture_task();
        t.route_spec.model = Some(pinned.into());
        let mut form = TaskEditForm::new(RUN_ID, &t);
        form.efforts = ["low", "medium", "high"].map(String::from).to_vec();
        focus(&mut form, EditField::Effort);
        form.on_key(key(KeyCode::Right));
        let PlanEdit::AmendTask { route, .. } = amend(&form) else {
            panic!("an amend");
        };
        assert_eq!(
            route,
            Some(RouteSpec {
                runtime: Some(Runtime::Claude),
                model: Some(pinned.into()),
                strength: None,
                effort: Some(Effort::HIGH),
            }),
            "an untouched model is the plan's own, hostile {hostile}"
        );
    }
}

#[test]
fn ctrl_j_outside_the_brief_does_nothing() {
    let mut t = edit_fixture_task();
    t.test_mode = TestMode::Check;
    t.test_mode_reason = Some("renames".into());
    let mut form = TaskEditForm::new(RUN_ID, &t);
    for field in [EditField::Model, EditField::Reason] {
        focus(&mut form, field);
        let before = form.clone();
        assert_eq!(form.on_key(ctrl('j')), EditOutcome::Stay);
        assert_eq!(form, before, "{field:?}");
    }
    assert_eq!(form.edits(), Ok(vec![]));
}

#[test]
fn a_reason_typed_then_back_to_tdd_is_nothing_changed() {
    let mut form = edit_fixture_form();
    focus(&mut form, EditField::TestMode);
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.test_mode, TestMode::Check);
    focus(&mut form, EditField::Reason);
    typed(&mut form, "x");
    focus(&mut form, EditField::TestMode);
    form.on_key(key(KeyCode::Left));
    assert_eq!(form.test_mode, TestMode::Tdd);
    assert_eq!(form.on_key(key(KeyCode::Enter)), EditOutcome::Unchanged);
    assert!(!form.submitting);
}

#[test]
fn a_resubmit_clears_the_refusal() {
    let mut form = edit_fixture_form();
    focus(&mut form, EditField::Size);
    form.on_key(key(KeyCode::Right));
    form.error = Some("task t1: size: below the floor".into());
    assert!(matches!(
        form.on_key(key(KeyCode::Enter)),
        EditOutcome::Submit(_)
    ));
    assert_eq!(form.error, None);
    assert!(form.submitting);
}

#[test]
fn after_a_new_model_the_role_table_rows_show_no_stale_resolved_value() {
    let mut form = edit_fixture_form();
    let sol = proto::models::ModelRef::parse("codex:gpt-6-sol").unwrap();
    form.choose(Some(sol), "Codex · gpt-6 sol".into());
    assert_eq!(
        form.value_parts(EditField::Model),
        ("‹ Codex · gpt-6 sol ›".into(), None)
    );
    assert_eq!(
        form.value_parts(EditField::Effort),
        ("‹ default ›".into(), None),
        "fix round 1: a named model with no effort runs at its default"
    );
    form.choose(None, String::new());
    assert_eq!(form.route, RouteSpec::default());
    assert_eq!(
        form.value_parts(EditField::Model),
        ("‹ role table (Claude · claude-sonnet-5) ›".into(), None)
    );
}

/// Final fix wave (the 13a ruling): the brief's cap is one million characters, which
/// keeps the edit form's per-key draw and insert well inside the 100 ms tick; a longer
/// brief still opens whole-or-refused, never cut and sent.
#[test]
fn the_brief_cap_is_a_million_characters() {
    assert_eq!(BRIEF_MAX_CHARS, 1_000_000);
}

/// Final fix wave (the 13a ruling): the largest edit the form can send — a brief at
/// the cap in 4-byte characters, and a model and a reason at `TEXT_MAX_CHARS` of them —
/// encodes within one frame.
#[test]
fn the_largest_edit_fits_one_frame() {
    let wide = |n: usize| "𝄞".repeat(n);
    let edit = PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: Some(wide(BRIEF_MAX_CHARS)),
        acceptance: None,
        route: Some(proto::RouteSpec {
            runtime: Some(proto::Runtime::Codex),
            model: Some(wide(TEXT_MAX_CHARS)),
            strength: None,
            effort: Some(proto::Effort::HIGH),
        }),
        test_mode: Some(proto::TestMode::None),
        test_mode_reason: Some(wide(TEXT_MAX_CHARS)),
        priority: Some(i32::MAX),
        size: Some(proto::Size::L),
        deps: None,
        stage: Some(u16::MAX),
        race: None,
        pair: None,
    };
    let msg = proto::ClientMsg::RunTagged {
        id: u64::MAX,
        request: proto::RunRequest::Edit {
            run_id: "r".repeat(64),
            edits: vec![edit],
            submit: false,
        },
    };
    let frame = proto::encode(&msg).expect("the edit fits one frame");
    assert!(frame.len() <= proto::MAX_FRAME + 4, "{} bytes", frame.len());
}
