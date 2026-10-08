//! M9.15: the goal form's own rules, apart from the app (decision 44), and 9.0.6
//! decision 39's toggles and model picker.

use super::*;
use crate::run_edit::TEXT_MAX_CHARS;
use proto::Runtime;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn form_with(goal: &str) -> GoalForm {
    let mut form = GoalForm::new("/r/demo".into());
    form.goal = TextArea::from_text(goal);
    form
}

/// The app with the fixture catalogs and `C-b g`'s form open on `/tmp`, its goal typed.
fn goal_app() -> crate::app::App {
    let mut app = crate::app::App::new(vec![], "/tmp".into(), Default::default());
    app.catalogs = crate::app::model_picker::tests::fixture_catalogs();
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.on_key(key(KeyCode::Char('g')));
    for c in "ship it".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    app
}

fn open_form(app: &crate::app::App) -> &GoalForm {
    match &app.modal {
        Some(crate::app::Modal::StartGoal(form)) => form,
        other => panic!("no goal form: {other:?}"),
    }
}

/// The `StartGoal` Ctrl-S sends.
fn started(app: &mut crate::app::App) -> RunRequest {
    let effects = app.on_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
    effects
        .into_iter()
        .find_map(|e| match e {
            crate::app::Effect::Send(proto::ClientMsg::RunTagged { request, .. }) => Some(request),
            _ => None,
        })
        .expect("a StartGoal")
}

fn orchestrator(request: RunRequest) -> Option<OrchestratorChoice> {
    match request {
        RunRequest::StartGoal { orchestrator, .. } => orchestrator,
        other => panic!("{other:?}"),
    }
}

/// Milestone 9.8 decision 39: the model row's `⏎` opens the picker, `role table (<the
/// orchestrator row>)` first; the effort row cycles the chosen model's catalog efforts
/// and `default`; the request carries both. `role table` sends no choice.
#[test]
fn the_goal_form_chooses_the_orchestrator_from_the_picker() {
    use crate::app::model_picker::tests::{mref, selected_model};
    use crate::app::model_picker::{PickerEntry, PickerFor};
    let mut app = goal_app();
    app.on_key(key(KeyCode::Tab));
    assert_eq!(open_form(&app).focus, GoalField::Model);
    assert!(
        app.on_key(key(KeyCode::Enter)).is_empty(),
        "⏎ opens, sends nothing"
    );
    let picker = open_form(&app).picker.as_ref().expect("the picker");
    assert_eq!(picker.target, PickerFor::Goal);
    assert_eq!(
        picker.entries[0],
        PickerEntry::RoleTable("role table (Claude · Opus 5.5)".into())
    );
    assert_eq!(picker.selected, 0, "no choice yet: on the role table");
    let sol = mref("codex:gpt-6.1-sol");
    for _ in 0..10 {
        if selected_model(open_form(&app).picker.as_ref().unwrap()) == Some(sol.clone()) {
            break;
        }
        app.on_key(key(KeyCode::Char('j')));
    }
    app.on_key(key(KeyCode::Enter));
    let form = open_form(&app);
    assert!(form.picker.is_none());
    assert_eq!(form.model, Some(sol));
    app.on_key(key(KeyCode::Tab));
    assert_eq!(open_form(&app).focus, GoalField::Effort);
    let mut seen = Vec::new();
    for _ in 0..5 {
        app.on_key(key(KeyCode::Char(' ')));
        seen.push(open_form(&app).effort.clone());
    }
    let want = ["low", "medium", "high", "max"].map(|e| Some(e.to_string()));
    assert_eq!(seen[..4], want);
    assert_eq!(seen[4], None, "then the model's default");
    app.on_key(key(KeyCode::Left));
    assert_eq!(
        orchestrator(started(&mut app)),
        Some(OrchestratorChoice {
            runtime: Runtime::Codex,
            model: Some("gpt-6.1-sol".into()),
            effort: Some("max".into()),
        })
    );

    // `role table`: no choice; the effort row has nothing to cycle.
    let mut app = goal_app();
    app.on_key(key(KeyCode::Tab));
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Enter));
    assert_eq!(open_form(&app).model, None);
    app.on_key(key(KeyCode::Tab));
    app.on_key(key(KeyCode::Char(' ')));
    assert_eq!(open_form(&app).effort, None);
    assert_eq!(orchestrator(started(&mut app)), None);
}

/// `esc` in the picker closes it and keeps the form; `r` asks for a live probe.
#[test]
fn the_goal_forms_picker_cancels_and_refreshes() {
    let mut app = goal_app();
    app.on_key(key(KeyCode::Tab));
    app.on_key(key(KeyCode::Enter));
    assert_eq!(
        app.on_key(key(KeyCode::Char('r'))),
        [crate::app::Effect::Send(proto::ClientMsg::ListModels {
            runtime: None,
            refresh: true
        })]
    );
    app.on_key(key(KeyCode::Esc));
    let form = open_form(&app);
    assert!(form.picker.is_none(), "esc closes the picker only");
    assert_eq!(form.focus, GoalField::Model);
}

#[test]
fn a_goal_paste_keeps_its_lines_and_drops_controls() {
    let mut form = form_with("");
    form.on_paste("one\r\ntwo\x1b[31m\u{202E}");
    assert_eq!(form.goal.text(), "one\ntwo[31m");
    form.goal = TextArea::new();
    form.on_paste(&"y".repeat(TEXT_MAX_CHARS + 10));
    assert_eq!(form.goal.text().chars().count(), TEXT_MAX_CHARS);
}

#[test]
fn ctrl_j_is_a_newline_in_the_goal() {
    let mut form = form_with("a");
    form.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    form.on_key(key(KeyCode::Char('b')));
    assert_eq!(form.request().unwrap(), {
        let mut want = form_with("a\nb").request().unwrap();
        if let RunRequest::StartGoal { goal, .. } = &mut want {
            assert_eq!(goal, "a\nb");
        }
        want
    });
}

#[test]
fn space_toggles_trust_and_esc_cancels_even_while_submitting() {
    let mut form = form_with("add a");
    form.focus = GoalField::Trust;
    form.on_key(key(KeyCode::Char(' ')));
    assert!(form.trust_project);
    assert!(matches!(
        form.on_key(key(KeyCode::Enter)),
        GoalOutcome::Submit(_)
    ));
    assert!(form.submitting);
    assert_eq!(form.on_key(key(KeyCode::Enter)), GoalOutcome::Stay);
    assert_eq!(form.on_key(key(KeyCode::Esc)), GoalOutcome::Cancel);
}

/// Milestone 9.3 (changed expectation): eight fields, the orchestrator row after the
/// model's. Milestone 9.8 decision 39 (changed expectation): `model` and `effort` in
/// place of `runtime` and `model`. Milestone 9.6 task 18 (changed expectation, renamed from `…_eight_…`): nine,
/// the design row after delivery.
#[test]
fn tab_visits_the_nine_fields_in_the_dialogs_order() {
    let mut form = form_with("add a");
    let mut seen = vec![form.focus];
    for _ in 0..8 {
        form.on_key(key(KeyCode::Tab));
        seen.push(form.focus);
    }
    assert_eq!(
        seen,
        [
            GoalField::Goal,
            GoalField::Model,
            GoalField::Effort,
            GoalField::Orchestrator,
            GoalField::Delivery,
            GoalField::Design,
            GoalField::Trust,
            GoalField::Yes,
            GoalField::UnconfinedChecks,
        ]
    );
    form.on_key(key(KeyCode::Tab));
    assert_eq!(form.focus, GoalField::Goal);
    assert_eq!(
        seen.iter().map(|f| field_label(*f)).collect::<Vec<_>>(),
        [
            "goal",
            "model",
            "effort",
            "orchestrator",
            "delivery",
            "design",
            "trust",
            "approve at once",
            "unconfined checks"
        ]
    );
}

/// Milestone 9.2 ruling R-13: the delivery choice cycles `configured`, `local`, `pr`
/// both ways, and the request carries it; `configured` sends none, so the daemon reads
/// the repo profile.
#[test]
fn the_delivery_choice_cycles_and_is_sent() {
    let mut form = form_with("add a");
    form.focus = GoalField::Delivery;
    let delivery = |form: &GoalForm| match form.request().unwrap() {
        RunRequest::StartGoal { delivery, .. } => delivery,
        other => panic!("{other:?}"),
    };
    assert_eq!(delivery(&form), None);
    let mut seen = Vec::new();
    for code in [
        KeyCode::Right,
        KeyCode::Char(' '),
        KeyCode::Right,
        KeyCode::Left,
    ] {
        form.on_key(key(code));
        seen.push(delivery(&form));
    }
    assert_eq!(
        seen,
        [
            Some(DeliveryMode::Local),
            Some(DeliveryMode::Pr),
            None,
            Some(DeliveryMode::Pr),
        ]
    );
}

/// Milestone 9.6 task 18 (DF §6.2): the `design` row, after `delivery`. It starts at
/// `configured`, which sends no mode, so the daemon decides from
/// `[orchestrator.design].default` and DF §1's table (an explicit `full` on a goal
/// triaged fast would be refused, ruling T3-1); it cycles `configured`, `full`, `off`
/// both ways, and the request carries the choice.
#[test]
fn the_goal_dialog_has_a_design_row_defaulting_from_config() {
    let mut form = form_with("add a");
    let design = |form: &GoalForm| match form.request().unwrap() {
        RunRequest::StartGoal { design, .. } => design,
        other => panic!("{other:?}"),
    };
    assert_eq!(design(&form), None);
    form.focus = GoalField::Delivery;
    form.on_key(key(KeyCode::Tab));
    assert_eq!(form.focus, GoalField::Design);
    assert_eq!(field_label(GoalField::Design), "design");
    let mut seen = Vec::new();
    for code in [
        KeyCode::Right,
        KeyCode::Char(' '),
        KeyCode::Right,
        KeyCode::Left,
    ] {
        form.on_key(key(code));
        seen.push(design(&form));
    }
    use proto::DesignMode::{Full, Off};
    assert_eq!(seen, [Some(Full), Some(Off), None, Some(Off)]);
    // The row reads the choice; `configured` while none is made.
    let row = |form: &GoalForm| {
        crate::ui::run_goal::option_lines(form, 60, crate::theme::Palette::PLAIN)
            .iter()
            .map(|line| line.to_string())
            .find(|line| line.contains("design"))
            .expect("a design row")
    };
    assert_eq!(row(&form).trim_end(), "▌ design            ‹ off ›");
    form.design = None;
    form.focus = GoalField::Goal;
    assert_eq!(row(&form).trim_end(), "  design            ‹ configured ›");
    // Ruling T18-2: with the settings' default known, `configured` names it, and still
    // sends no mode.
    form.design_default = Some(proto::DesignMode::Full);
    assert_eq!(
        row(&form).trim_end(),
        "  design            ‹ configured (full) ›"
    );
    assert_eq!(design(&form), None);
}
