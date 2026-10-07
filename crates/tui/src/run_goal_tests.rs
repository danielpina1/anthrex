//! M9.15: the goal form's own rules, apart from the app (decision 44), and 9.0.6
//! decision 39's toggles and model picker.

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn form_with(goal: &str) -> GoalForm {
    let mut form = GoalForm::new("/r/demo".into());
    form.goal = TextArea::from_text(goal);
    form
}

fn entry(runtime: Runtime, model: &str) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.into(),
        strength: proto::Strength::Standard,
        note: String::new(),
    }
}

/// M9.0.6: with the runtime configured the picker holds `default` only, so a model can
/// no longer be named without a runtime (the old `MODEL_NEEDS_RUNTIME` is gone); a
/// choice of runtime clears the model, which named another's.
#[test]
fn a_model_belongs_to_a_runtime() {
    let mut form = form_with("add a");
    form.set_roster(vec![entry(Runtime::Claude, "claude-opus-5-5")]);
    form.focus = GoalField::Model;
    form.on_paste("gpt-5");
    assert_eq!(form.model, GoalModel::Default, "a paste has nowhere to go");
    assert_eq!(form.model_options(), ["default"]);
    form.focus = GoalField::Runtime;
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.runtime, Some(Runtime::Claude));
    form.focus = GoalField::Model;
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.chosen_model().as_deref(), Some("claude-opus-5-5"));
    form.focus = GoalField::Runtime;
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.model, GoalModel::Default);
    assert_eq!(form.chosen_model(), None);
}

#[test]
fn codex_with_no_model_is_its_configured_default() {
    let mut form = form_with("  add a  ");
    form.runtime = Some(Runtime::Codex);
    assert_eq!(
        form.request(),
        Ok(RunRequest::StartGoal {
            goal: "add a".into(),
            dir: "/r/demo".into(),
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: Some(OrchestratorChoice {
                runtime: Runtime::Codex,
                model: None,
                effort: None,
            }),
            delivery: None,
            continue_from: None,
            design: None,
        })
    );
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
/// model's. Milestone 9.6 task 18 (changed expectation, renamed from `…_eight_…`): nine,
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
            GoalField::Runtime,
            GoalField::Model,
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
            "runtime",
            "model",
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
